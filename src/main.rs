use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use std::{env, thread};

use image::imageops::FilterType;
use image::DynamicImage;
use log::info;
use speedy2d::color::Color;
use speedy2d::dimen::{UVec2, Vec2};
use speedy2d::font::Font;
use speedy2d::image::ImageHandle;
use speedy2d::window::{KeyScancode, UserEventSender, VirtualKeyCode, WindowHandler, WindowHelper};
use speedy2d::{Graphics2D, Window};
use sqlite::Connection;
use thiserror::Error;

mod db;
mod disk;
mod draw;
mod metadata;

#[derive(Error, Debug)]
pub enum Error {
    #[error("std io error: {0:?}")]
    Io(#[from] std::io::Error),
    #[error("image error: {0:?}")]
    Image(#[from] image::ImageError),
    #[error("log error: {0:?}")]
    Log(#[from] log::SetLoggerError),
    #[error("sqlite error: {0:?}")]
    Sqlite(#[from] sqlite::Error),
    #[error("expected 2 args")]
    InvalidArgs,
    #[error("exif error: {0:?}")]
    Exif(#[from] exif::Error),
    #[error("no image metadata")]
    NoImageMetadata,
}

#[derive(Debug)]
pub struct ImageNamePair {
    /// name of the jpg file e.g. "IMG_0771.JPG"
    pub jpg_file_name: String,
    /// for example .cr2 raw files with the same name as the jpg
    /// e.g. vec!["IMG_0771.CR2"]
    pub other_file_names: Vec<String>,
    pub is_starred: bool,
    pub date_time: SystemTime,
}

struct Images {
    inner: Vec<ImageNamePair>,
    index: usize,
}

impl Images {
    fn new(name: &str, image_file_names: Vec<ImageNamePair>) -> Self {
        let index = Self::get_image_index(name, &image_file_names);
        Self {
            inner: image_file_names,
            index,
        }
    }

    fn get_image_index(name: &str, image_file_names: &[ImageNamePair]) -> usize {
        for (i, image_name) in image_file_names.iter().enumerate() {
            if name == image_name.jpg_file_name {
                return i;
            }
        }

        0
    }

    fn next(&mut self) {
        if self.index == self.inner.len() - 1 {
            self.index = 0
        } else {
            self.index += 1
        }
    }

    fn prev(&mut self) {
        if self.index == 0 {
            self.index = self.inner.len() - 1
        } else {
            self.index -= 1;
        }
    }

    fn current(&self) -> &ImageNamePair {
        &self.inner[self.index]
    }

    fn current_mut(&mut self) -> &mut ImageNamePair {
        &mut self.inner[self.index]
    }

    fn current_index(&self) -> usize {
        self.index
    }

    fn all(&self) -> &Vec<ImageNamePair> {
        &self.inner
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RenderState {
    Zooming,
    Full,
    LoadingFull,
    ExportRequested,
    Exporting,
    Help,
    Metadata,
}

fn main() -> Result<(), Error> {
    simple_logger::SimpleLogger::new().init()?;

    let args: Vec<String> = env::args().collect();
    info!("Args: {args:?}");
    if args.len() != 2 {
        return Err(Error::InvalidArgs);
    }

    let file_name = &args[1];
    let path = PathBuf::from(file_name);

    let name = path
        .file_name()
        .expect("not a valid file")
        .to_str()
        .expect("empty file name");

    let path = path
        .parent()
        .expect("not a valid folder")
        .to_str()
        .expect("empty parent folder")
        .to_owned();

    info!("Working folder: {path}");

    let connection = Arc::new(Mutex::new(db::get_or_create_db(&path)?));
    let image_file_names = build_file_list(&path, connection.clone())?;
    if image_file_names.is_empty() {
        // no images exit early
        info!("No images");
        return Ok(());
    }
    let images = Images::new(name, image_file_names);

    let window = Window::new_fullscreen_borderless("Image Viewer").expect("cannot create window");
    let screen_resolution = UVec2 { x: 800, y: 600 };
    let font = Font::new(include_bytes!("../fonts/NotoSans-Regular.ttf")).unwrap();
    let progress_percentage = Arc::new(AtomicI32::new(100));
    let user_event_sender = Arc::new(Mutex::new(window.create_user_event_sender()));

    window.run_loop(PhotoWindowHandler {
        image: None,
        images,
        screen_resolution,
        connection,
        path,
        state: RenderState::Full,
        font,
        progress_percentage,
        user_event_sender,
    })
}

fn build_file_list(
    path: &str,
    connection: Arc<Mutex<Connection>>,
) -> Result<Vec<ImageNamePair>, Error> {
    let mut image_file_names = disk::get_file_names(path)?;

    let names = db::get_starred_image_names(connection)?;
    for file in image_file_names.iter_mut() {
        if names.contains(&file.jpg_file_name) {
            file.is_starred = true;
        }
    }

    Ok(image_file_names)
}

#[allow(clippy::too_many_arguments)]
fn update_cache_image(
    path: &str,
    image_index: usize,
    num_images: usize,
    size: UVec2,
    connection: Arc<Mutex<Connection>>,
    name: &str,
    num_processed: &mut usize,
    progress_percentage: Arc<AtomicI32>,
    user_event_sender: Arc<Mutex<UserEventSender<()>>>,
) -> Result<(), Error> {
    info!(
        "Resizing image {} of {}: {}",
        image_index + 1,
        num_images,
        name
    );
    if db::photo_exists(name, size, connection.clone())? {
        info!("Photo already exists, skipping...");
    } else {
        load_and_insert_image(path, name, size, connection)?;
    }

    *num_processed += 1;
    let percentage = (100.0 * *num_processed as f64 / num_images as f64).ceil() as i32;
    progress_percentage.store(percentage, Ordering::Relaxed);
    let locked = user_event_sender.lock().unwrap();
    locked.send_event(()).unwrap();
    Ok(())
}

fn load_and_insert_image(
    path: &str,
    name: &str,
    size: UVec2,
    connection: Arc<Mutex<Connection>>,
) -> Result<Vec<u8>, Error> {
    let img = load_image(path, name)?;
    let resized = resize_jpg(&img, size)?;
    db::insert_image(name, size, &resized, connection)?;
    Ok(resized)
}

pub fn calculate_position_middle(screen_resolution: UVec2, image: &ImageHandle) -> Vec2 {
    let x = (screen_resolution.x - image.size().x) as f32 / 2.0;
    let y = (screen_resolution.y - image.size().y) as f32 / 2.0;
    Vec2 { x, y }
}

fn update_cache(
    path: String,
    image_file_names: Vec<String>,
    image_index: usize,
    size: UVec2,
    connection: Arc<Mutex<Connection>>,
    progress_percentage: Arc<AtomicI32>,
    user_event_sender: Arc<Mutex<UserEventSender<()>>>,
) -> Result<(), Error> {
    let mut num_processed = 0;
    // start resizing from one after the current photo (so we don't duplicate effort on startup)
    for (i, name) in image_file_names.iter().enumerate().skip(image_index + 1) {
        update_cache_image(
            &path,
            i,
            image_file_names.len(),
            size,
            connection.clone(),
            name,
            &mut num_processed,
            progress_percentage.clone(),
            user_event_sender.clone(),
        )?
    }

    // continue resizing from start
    for (i, name) in image_file_names.iter().enumerate().take(image_index + 1) {
        update_cache_image(
            &path,
            i,
            image_file_names.len(),
            size,
            connection.clone(),
            name,
            &mut num_processed,
            progress_percentage.clone(),
            user_event_sender.clone(),
        )?
    }

    info!("Done resizing");
    Ok(())
}

fn load_image(path: &str, name: &str) -> Result<DynamicImage, Error> {
    let file_name = disk::get_full_path(path, name);
    let file = File::open(file_name)?;
    let reader = BufReader::new(&file);
    let img = image::load(reader, image::ImageFormat::Jpeg).unwrap();

    let metadata = metadata::get_metadata(path, name)?;
    info!("{:?}", metadata);

    // rotate image if it contains exif metadata to do so
    let img = match metadata.orientation {
        Some(8) => img.rotate270(),
        Some(3) => img.rotate180(),
        Some(6) => img.rotate90(),
        _ => img, // do nothing
    };

    Ok(img)
}

fn crop_center(img: DynamicImage, size: UVec2) -> Result<DynamicImage, Error> {
    let width = size.x;
    let height = size.y;

    if width < img.width() && height < img.height() {
        let x = img.width() / 2 - width / 2;
        let y = img.height() / 2 - height / 2;
        let img = img.crop_imm(x, y, width, height);
        Ok(img)
    } else {
        Ok(img)
    }
}

fn resize_jpg(img: &DynamicImage, size: UVec2) -> Result<Vec<u8>, Error> {
    // this takes a long time
    let resized = img.resize(size.x, size.y, FilterType::CatmullRom); // cubic filter
    let buf = encode_jpg(&resized)?;
    Ok(buf)
}

fn encode_jpg(img: &DynamicImage) -> Result<Vec<u8>, Error> {
    let mut buf_out: Vec<u8> = Vec::new();

    {
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf_out, 90);
        encoder.encode_image(img)?;
    }

    Ok(buf_out)
}

fn resolution_ok(screen_resolution: UVec2) -> bool {
    screen_resolution.x > 800
}

fn export(path: &str, image_file_names: &[ImageNamePair]) -> Result<(), Error> {
    let starred_images: Vec<&ImageNamePair> =
        image_file_names.iter().filter(|x| x.is_starred).collect();
    disk::export(path, &starred_images)?;
    Ok(())
}

struct PhotoWindowHandler {
    image: Option<ImageHandle>,
    images: Images,
    //  image_index: usize,
    //   image_file_names: Vec<ImageNamePair>,
    screen_resolution: UVec2,
    connection: Arc<Mutex<Connection>>,
    path: String,
    state: RenderState,
    font: Font,
    progress_percentage: Arc<AtomicI32>,
    user_event_sender: Arc<Mutex<UserEventSender<()>>>,
}

impl WindowHandler for PhotoWindowHandler {
    fn on_user_event(&mut self, helper: &mut WindowHelper<()>, _user_event: ()) {
        helper.request_redraw()
    }

    fn on_resize(&mut self, _helper: &mut WindowHelper<()>, size_pixels: UVec2) {
        log::info!("Screen resolution changed to: {size_pixels:?}");
        self.screen_resolution = size_pixels;

        // a little trick so that we dont resize on the first call to this on_size function which normally has the generic resolution of 800x600
        if resolution_ok(size_pixels) {
            let image_file_names = self
                .images
                .all()
                .iter()
                .map(|x| x.jpg_file_name.clone())
                .collect();
            let size = size_pixels;
            let connection = self.connection.clone();
            let path = self.path.clone();
            let image_index = self.images.current_index();
            let progress_percentage = self.progress_percentage.clone();
            let user_event_sender = self.user_event_sender.clone();

            thread::spawn(move || {
                update_cache(
                    path,
                    image_file_names,
                    image_index,
                    size,
                    connection,
                    progress_percentage,
                    user_event_sender,
                )
            });
            self.image = None;
        }
    }

    fn on_draw(&mut self, helper: &mut WindowHelper, graphics: &mut Graphics2D) {
        graphics.clear_screen(Color::BLACK);

        if resolution_ok(self.screen_resolution) {
            let image_file = self.images.current();
            let name = image_file.jpg_file_name.as_str();

            if self.image.is_none() {
                match self.state {
                    RenderState::Full => {
                        helper.set_cursor_visible(false);

                        match db::try_get_image_from_db(
                            name,
                            self.screen_resolution,
                            self.connection.clone(),
                        )
                        .unwrap()
                        {
                            Some(db_image) => {
                                let image =
                                    draw::image(self.screen_resolution, &db_image, graphics);
                                self.image = Some(image);
                            }
                            None => {
                                // draw an hourglass to the screen to indicate loading
                                let image_bytes = include_bytes!("../img/hourglass.jpg");
                                draw::image(self.screen_resolution, image_bytes, graphics);
                                helper.request_redraw();
                                self.state = RenderState::LoadingFull;
                            }
                        }
                    }
                    RenderState::Zooming => {
                        helper.set_cursor_visible(true);
                        let img = load_image(&self.path, name).unwrap();
                        let img = crop_center(img, self.screen_resolution).unwrap();
                        draw::image_full(img, graphics);
                    }
                    RenderState::LoadingFull => {
                        let resized = load_and_insert_image(
                            &self.path,
                            name,
                            self.screen_resolution,
                            self.connection.clone(),
                        )
                        .unwrap();

                        let image = draw::image(self.screen_resolution, &resized, graphics);
                        self.image = Some(image);
                        self.state = RenderState::Full;
                    }
                    RenderState::ExportRequested => {
                        let image_bytes = include_bytes!("../img/hourglass.jpg");
                        draw::image(self.screen_resolution, image_bytes, graphics);
                        helper.request_redraw();
                        self.state = RenderState::Exporting;
                    }
                    RenderState::Exporting => {
                        export(&self.path, self.images.all()).unwrap();
                        self.state = RenderState::Full;
                        helper.request_redraw();
                    }
                    RenderState::Help => draw::help(self.screen_resolution, graphics, &self.font),
                    RenderState::Metadata => {
                        let metadata = metadata::get_metadata(&self.path, name).unwrap();
                        draw::metadata(
                            name,
                            self.screen_resolution,
                            graphics,
                            &self.font,
                            &metadata,
                        )
                    }
                }
            } else {
                let image = self.image.as_ref().expect("no image set");
                let position = calculate_position_middle(self.screen_resolution, image);
                graphics.draw_image(position, image);
            }

            if image_file.is_starred {
                draw::star(self.screen_resolution, graphics);
            }

            draw::progress_text(
                self.screen_resolution,
                graphics,
                &self.font,
                self.progress_percentage.clone(),
            );
        }
    }

    fn on_key_down(
        &mut self,
        helper: &mut WindowHelper,
        virtual_key_code: Option<VirtualKeyCode>,
        scancode: KeyScancode,
    ) {
        match virtual_key_code {
            Some(VirtualKeyCode::Escape) => match self.state {
                RenderState::Help | RenderState::Metadata => {
                    self.state = RenderState::Full;
                    helper.request_redraw()
                }
                _ => std::process::exit(0),
            },
            Some(VirtualKeyCode::Left) => {
                // prev image
                self.images.prev();
                self.image = None;
                helper.request_redraw();
            }
            Some(VirtualKeyCode::Right) => {
                // next image
                self.images.next();
                self.image = None;
                helper.request_redraw();
            }
            Some(VirtualKeyCode::LControl) => {
                self.state = RenderState::Zooming;
                self.image = None;
                helper.request_redraw();
            }
            Some(VirtualKeyCode::Space) => {
                let image = self.images.current_mut();
                image.is_starred = !image.is_starred;
                db::update_image_is_starred(
                    &image.jpg_file_name,
                    image.is_starred,
                    self.connection.clone(),
                )
                .unwrap();
                helper.request_redraw();
            }
            Some(VirtualKeyCode::E) if self.state != RenderState::ExportRequested => {
                self.state = RenderState::ExportRequested;
                self.image = None;
                helper.request_redraw();
            }
            Some(VirtualKeyCode::F1) => {
                if self.state == RenderState::Help {
                    self.state = RenderState::Full;
                } else {
                    self.state = RenderState::Help;
                }
                self.image = None;
                helper.request_redraw()
            }
            Some(VirtualKeyCode::F3) => {
                if self.state == RenderState::Metadata {
                    self.state = RenderState::Full;
                } else {
                    self.state = RenderState::Metadata;
                }
                self.image = None;
                helper.request_redraw()
            }
            _ => {}
        }

        log::info!(
            "Got on_key_down callback: {:?}, scancode {}",
            virtual_key_code,
            scancode
        );
    }

    fn on_key_up(
        &mut self,
        helper: &mut WindowHelper<()>,
        virtual_key_code: Option<VirtualKeyCode>,
        _scancode: KeyScancode,
    ) {
        if let Some(VirtualKeyCode::LControl) = virtual_key_code {
            self.state = RenderState::Full;
            self.image = None;
            helper.request_redraw();
        }
    }
}
