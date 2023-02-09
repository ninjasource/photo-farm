use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::{env, thread};

use exif::{In, Tag};
use image::imageops::FilterType;
use image::DynamicImage;
use log::info;
use speedy2d::color::Color;
use speedy2d::dimen::{UVec2, Vec2};
use speedy2d::font::{Font, TextAlignment, TextLayout, TextOptions};
use speedy2d::image::{ImageDataType, ImageHandle, ImageSmoothingMode};
use speedy2d::window::{KeyScancode, UserEventSender, VirtualKeyCode, WindowHandler, WindowHelper};
use speedy2d::{Graphics2D, Window};
use sqlite::Connection;
use thiserror::Error;

mod db;
mod disk;

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
}

struct ImageMetadata {
    orientation: Option<u32>,
}

#[derive(Debug, PartialEq, Eq)]
enum RenderState {
    Zooming,
    Full,
    LoadingFull,
    ExportRequested,
    Exporting,
    Help,
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
    let image_file_names = Arc::new(disk::get_file_names(&path)?);
    if image_file_names.len() == 0 {
        // no images exit early
        info!("No images");
        return Ok(());
    }

    let window = Window::new_fullscreen_borderless("Image Viewer").expect("cannot create window");
    let screen_resolution = UVec2 { x: 800, y: 600 };
    let image_index = get_image_index(name, &image_file_names);

    let font = Font::new(include_bytes!("../fonts/NotoSans-Regular.ttf")).unwrap();

    let progress_percentage = Arc::new(AtomicI32::new(100));

    let user_event_sender = Arc::new(Mutex::new(window.create_user_event_sender()));

    window.run_loop(MyWindowHandler {
        image: None,
        image_index,
        image_file_names,
        screen_resolution,
        connection,
        path,
        state: RenderState::Full,
        font,
        is_starred: false,
        progress_percentage,
        user_event_sender,
    })
}

fn get_image_index(name: &str, image_file_names: &Vec<ImageNamePair>) -> usize {
    for (i, image_name) in image_file_names.iter().enumerate() {
        if name == image_name.jpg_file_name {
            return i;
        }
    }

    return 0;
}

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
    db::insert_image(&name, size, &resized, connection)?;
    Ok(resized)
}

fn update_cache(
    path: String,
    image_file_names: Arc<Vec<ImageNamePair>>,
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
            &name.jpg_file_name,
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
            &name.jpg_file_name,
            &mut num_processed,
            progress_percentage.clone(),
            user_event_sender.clone(),
        )?
    }

    info!("Done resizing");
    Ok(())
}

fn get_metadata(path: &str, name: &str) -> Result<ImageMetadata, Error> {
    let file_name = disk::get_full_path(path, name);
    let file = File::open(&file_name)?;
    let mut reader = BufReader::new(&file);
    let exif = exif::Reader::new().read_from_container(&mut reader)?;

    let orientation = if let Some(field) = exif.get_field(Tag::Orientation, In::PRIMARY) {
        field.value.get_uint(0)
    } else {
        None
    };

    Ok(ImageMetadata { orientation })
}

fn load_image(path: &str, name: &str) -> Result<DynamicImage, Error> {
    let file_name = disk::get_full_path(path, name);
    let file = File::open(&file_name)?;
    let reader = BufReader::new(&file);
    let img = image::load(reader, image::ImageFormat::Jpeg).unwrap();

    // rotate image if it contains exif metadata to do so
    let img = match get_metadata(path, name) {
        Ok(ImageMetadata {
            orientation: Some(8),
        }) => img.rotate270(),
        Ok(ImageMetadata {
            orientation: Some(3),
        }) => img.rotate180(),
        Ok(ImageMetadata {
            orientation: Some(6),
        }) => img.rotate90(),
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

fn calculate_position_middle(screen_resolution: UVec2, image: &ImageHandle) -> Vec2 {
    let x = (screen_resolution.x - image.size().x) as f32 / 2.0;
    let y = (screen_resolution.y - image.size().y) as f32 / 2.0;
    Vec2 { x, y }
}

fn draw_star(size: UVec2, graphics: &mut Graphics2D) {
    let image_bytes = include_bytes!("../img/star_24px.png");
    let file_bytes = Cursor::new(image_bytes);
    let image = graphics
        .create_image_from_file_bytes(None, ImageSmoothingMode::NearestNeighbor, file_bytes)
        .unwrap(); // complicated error struct
    let position = Vec2 {
        x: size.x as f32 - image.size().x as f32 - 10.0,
        y: 10.0,
    };

    graphics.draw_image(position, &image);
}

fn draw_image(size: UVec2, file_bytes: &[u8], graphics: &mut Graphics2D) -> ImageHandle {
    let file_bytes = Cursor::new(file_bytes);
    let image = graphics
        .create_image_from_file_bytes(None, ImageSmoothingMode::NearestNeighbor, file_bytes)
        .unwrap(); // complicated error struct
    let position = calculate_position_middle(size, &image);
    graphics.draw_image(position, &image);
    image
}

fn draw_progress_text(
    size: UVec2,
    graphics: &mut Graphics2D,
    font: &Font,
    progress_percentage: Arc<AtomicI32>,
) {
    let percentage = progress_percentage.load(Ordering::Relaxed);

    // only draw progress below 100 percent
    if percentage < 100 {
        let percentage = format!("{percentage} %",);

        let text = font.layout_text(
            &percentage,
            20.0,
            TextOptions::new().with_wrap_to_width(200.0, TextAlignment::Left),
        );

        graphics.draw_text(
            Vec2 {
                x: size.x as f32 - text.width() - 10.0,
                y: size.y as f32 - text.height() - 10.0,
            },
            Color::from_rgb(0.9, 0.9, 0.8),
            &text,
        );
    }
}

fn draw_image_full(img: DynamicImage, graphics: &mut Graphics2D) -> ImageHandle {
    let size = UVec2 {
        x: img.width(),
        y: img.height(),
    };
    let image = graphics
        .create_image_from_raw_pixels(
            ImageDataType::RGB,
            ImageSmoothingMode::NearestNeighbor,
            size,
            img.as_bytes(),
        )
        .unwrap(); // complicated error struct

    graphics.draw_image(Vec2 { x: 0.0, y: 0.0 }, &image);
    image
}

fn export(
    path: &str,
    image_file_names: &Vec<ImageNamePair>,
    connection: Arc<Mutex<Connection>>,
) -> Result<(), Error> {
    let names = db::get_starred_image_names(connection)?;
    let starred_images: Vec<&ImageNamePair> = image_file_names
        .into_iter()
        .filter(|x| names.contains(&x.jpg_file_name))
        .collect();
    disk::export(path, &starred_images)?;
    Ok(())
}

fn draw_help(size: UVec2, graphics: &mut Graphics2D, font: &Font) {
    let text = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        "F1", "SPACE", "LEFT CTRL", "ESC", "LEFT", "RIGHT", "E", "S",
    );
    let left_text = font.layout_text(
        &text,
        20.0,
        TextOptions::new().with_wrap_to_width(600.0, TextAlignment::Left),
    );

    let text = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        "Toggle help",
        "Toggle star",
        "Hold to zoom in to 1:1",
        "Exit",
        "Previous photo",
        "Next photo",
        "Export starred photos to 'export' folder",
        "Toggle show starred photos only"
    );

    let right_text = font.layout_text(
        &text,
        20.0,
        TextOptions::new().with_wrap_to_width(600.0, TextAlignment::Left),
    );

    let x_gap = 100.0;
    let x = size.x as f32 / 2.0 - (left_text.width() + right_text.width() + x_gap) / 2.0;
    let y = size.y as f32 / 2.0 - left_text.height() / 2.0;

    graphics.draw_text(Vec2 { x, y }, Color::from_rgb(0.9, 0.9, 0.8), &left_text);
    graphics.draw_text(
        Vec2 { x: x + x_gap, y },
        Color::from_rgb(0.6, 0.6, 0.5),
        &right_text,
    );
}

struct MyWindowHandler {
    image: Option<ImageHandle>,
    image_index: usize,
    image_file_names: Arc<Vec<ImageNamePair>>,
    screen_resolution: UVec2,
    connection: Arc<Mutex<Connection>>,
    path: String,
    state: RenderState,
    font: Font,
    is_starred: bool,
    progress_percentage: Arc<AtomicI32>,
    user_event_sender: Arc<Mutex<UserEventSender<()>>>,
}

impl WindowHandler for MyWindowHandler {
    fn on_user_event(&mut self, helper: &mut WindowHelper<()>, _user_event: ()) {
        helper.request_redraw()
    }

    fn on_resize(&mut self, _helper: &mut WindowHelper<()>, size_pixels: UVec2) {
        log::info!("Screen resolution changed to: {size_pixels:?}");
        self.screen_resolution = size_pixels;

        // a little trick so that we dont resize on the first call to this on_size function which normally has the generic resolution of 800x600
        if resolution_ok(size_pixels) {
            let image_file_names = self.image_file_names.clone();
            let size = size_pixels;
            let connection = self.connection.clone();
            let path = self.path.clone();
            let image_index = self.image_index;
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
            let name = self.image_file_names[self.image_index]
                .jpg_file_name
                .as_str();

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
                                    draw_image(self.screen_resolution, &db_image.resized, graphics);
                                self.image = Some(image);
                                self.is_starred = db_image.is_starred;
                            }
                            None => {
                                // draw an hourglass to the screen to indicate loading
                                let image_bytes = include_bytes!("../img/hourglass.jpg");
                                draw_image(self.screen_resolution, image_bytes, graphics);
                                helper.request_redraw();
                                self.state = RenderState::LoadingFull;
                            }
                        }
                    }
                    RenderState::Zooming => {
                        helper.set_cursor_visible(true);
                        let img = load_image(&self.path, name).unwrap();
                        let img = crop_center(img, self.screen_resolution).unwrap();
                        draw_image_full(img, graphics);
                    }
                    RenderState::LoadingFull => {
                        let resized = load_and_insert_image(
                            &self.path,
                            name,
                            self.screen_resolution,
                            self.connection.clone(),
                        )
                        .unwrap();

                        let image = draw_image(self.screen_resolution, &resized, graphics);
                        self.image = Some(image);
                        self.state = RenderState::Full;
                    }
                    RenderState::ExportRequested => {
                        let image_bytes = include_bytes!("../img/hourglass.jpg");
                        draw_image(self.screen_resolution, image_bytes, graphics);
                        helper.request_redraw();
                        self.state = RenderState::Exporting;
                    }
                    RenderState::Exporting => {
                        export(&self.path, &self.image_file_names, self.connection.clone())
                            .unwrap();
                        self.state = RenderState::Full;
                        helper.request_redraw();
                    }
                    RenderState::Help => draw_help(self.screen_resolution, graphics, &self.font),
                }
            } else {
                let image = self.image.as_ref().expect("no image set");
                let position = calculate_position_middle(self.screen_resolution, image);
                graphics.draw_image(position, image);
            }

            if self.is_starred {
                draw_star(self.screen_resolution, graphics);
            }

            draw_progress_text(
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
            Some(VirtualKeyCode::Escape) => {
                if self.state == RenderState::Help {
                    self.state = RenderState::Full;
                    helper.request_redraw()
                } else {
                    std::process::exit(0)
                }
            }
            Some(VirtualKeyCode::Left) => {
                // prev image
                if self.image_index == 0 {
                    self.image_index = self.image_file_names.len() - 1
                } else {
                    self.image_index -= 1;
                }
                self.image = None;
                self.is_starred = false;
                helper.request_redraw();
            }
            Some(VirtualKeyCode::Right) => {
                // next image
                if self.image_index == self.image_file_names.len() - 1 {
                    self.image_index = 0
                } else {
                    self.image_index += 1
                }
                self.image = None;
                self.is_starred = false;
                helper.request_redraw();
            }
            Some(VirtualKeyCode::LControl) => {
                self.state = RenderState::Zooming;
                self.image = None;
                self.is_starred = false;
                helper.request_redraw();
            }
            Some(VirtualKeyCode::Space) => {
                self.is_starred = !self.is_starred;
                let name = &self.image_file_names[self.image_index].jpg_file_name;
                db::update_image_is_starred(name, self.is_starred, self.connection.clone())
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
        match virtual_key_code {
            Some(VirtualKeyCode::LControl) => {
                self.state = RenderState::Full;
                self.image = None;
                helper.request_redraw();
            }
            _ => {}
        }
    }
}
