use image::DynamicImage;
use speedy2d::color::Color;
use speedy2d::dimen::{UVec2, Vec2};
use speedy2d::font::{Font, TextAlignment, TextLayout, TextOptions};
use speedy2d::image::{ImageDataType, ImageHandle, ImageSmoothingMode};
use speedy2d::Graphics2D;
use std::io::Cursor;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use crate::calculate_position_middle;
use crate::metadata::ImageMetadata;

pub fn file_name(graphics: &mut Graphics2D, file_name: &str, font: &Font) {
    let text = font.layout_text(
        file_name,
        20.0,
        TextOptions::new().with_wrap_to_width(200.0, TextAlignment::Left),
    );

    graphics.draw_text(
        Vec2 { x: 10.0, y: 5.0 },
        Color::from_rgb(0.9, 0.9, 0.8),
        &text,
    );
}

pub fn star(size: UVec2, graphics: &mut Graphics2D) {
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

pub fn image(size: UVec2, file_bytes: &[u8], graphics: &mut Graphics2D) -> ImageHandle {
    let file_bytes = Cursor::new(file_bytes);
    let image = graphics
        .create_image_from_file_bytes(None, ImageSmoothingMode::NearestNeighbor, file_bytes)
        .unwrap(); // complicated error struct
    let position = calculate_position_middle(size, &image);
    graphics.draw_image(position, &image);
    image
}

pub fn progress_text(
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

pub fn image_full(img: DynamicImage, graphics: &mut Graphics2D) -> ImageHandle {
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

pub fn metadata(
    name: &str,
    size: UVec2,
    graphics: &mut Graphics2D,
    font: &Font,
    metadata: &ImageMetadata,
) {
    let col0 = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        "File Name",
        "Camera Model",
        "Date Taken",
        "Exposure Time",
        "Aperture Value",
        "ISO Speed Rating",
        "Focal Length",
    );

    let col1 = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        name,
        metadata.model.as_deref().unwrap_or_default(),
        metadata.date_time.as_deref().unwrap_or_default(),
        metadata.exposure_time.as_deref().unwrap_or_default(),
        metadata.f_number.as_deref().unwrap_or_default(),
        metadata.iso.as_deref().unwrap_or_default(),
        metadata.focal_length.as_deref().unwrap_or_default(),
    );

    table(size, graphics, font, &col0, &col1);
}

fn table(size: UVec2, graphics: &mut Graphics2D, font: &Font, col0: &str, col1: &str) {
    let left_text = font.layout_text(
        col0,
        20.0,
        TextOptions::new().with_wrap_to_width(600.0, TextAlignment::Left),
    );

    let right_text = font.layout_text(
        col1,
        20.0,
        TextOptions::new().with_wrap_to_width(600.0, TextAlignment::Left),
    );

    let x_gap = 50.0;
    let x = size.x as f32 / 2.0 - (left_text.width() + right_text.width() + x_gap) / 2.0;
    let y = size.y as f32 / 2.0 - left_text.height() / 2.0;

    graphics.draw_text(Vec2 { x, y }, Color::from_rgb(0.9, 0.9, 0.8), &left_text);
    graphics.draw_text(
        Vec2 {
            x: x + left_text.width() + x_gap,
            y,
        },
        Color::from_rgb(0.6, 0.6, 0.5),
        &right_text,
    );
}

pub fn help(size: UVec2, graphics: &mut Graphics2D, font: &Font) {
    let col0 = format!(
        "{}\n\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        "Photo Farm", "F1", "F3", "SPACE", "LEFT CTRL", "ESC", "LEFT", "RIGHT", "E", "S", "I",
    );

    let col1 = format!(
        "{}\n\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        "An image viewer by David Haig",
        "Toggle help",
        "Toggle EXIF metadata",
        "Toggle star",
        "Hold to zoom in to 1:1",
        "Exit",
        "Previous photo",
        "Next photo",
        "Export starred photos to 'export' folder",
        "Toggle show starred photos only",
        "Toggle show file name",
    );

    table(size, graphics, font, &col0, &col1);
}
