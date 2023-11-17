use std::{fs::File, io::BufReader};

use chrono::NaiveDateTime;
use exif::{Exif, In, Tag};

use crate::{disk, Error};

#[derive(Debug)]
pub struct ImageMetadata {
    pub orientation: Option<u32>,
    pub iso: Option<String>,
    pub model: Option<String>,
    pub exposure_time: Option<String>,
    pub f_number: Option<String>,
    pub date_time: Option<String>,
    pub focal_length: Option<String>,
}

impl ImageMetadata {
    pub fn try_get_timestamp_from_date_time(&self) -> i64 {
        match self.date_time.as_deref() {
            Some(date_time) => string_to_unix_timestamp(date_time).unwrap_or(0),
            _ => 0,
        }
    }
}

fn string_to_unix_timestamp(s: &str) -> Result<i64, Error> {
    match NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        Ok(date_time) => Ok(date_time.timestamp()),
        Err(e) => Err(Error::ExifDateTime((s.to_owned(), e))),
    }
}

pub fn _get_date_time(path: &str, name: &str) -> Result<NaiveDateTime, Error> {
    let file_name = disk::get_full_path(path, name);
    let file = File::open(file_name)?;
    let mut reader = BufReader::new(&file);
    let exif = exif::Reader::new().read_from_container(&mut reader)?;

    match exif.get_field(Tag::DateTime, In::PRIMARY) {
        Some(field) => {
            let s = field.display_value().with_unit(&exif).to_string();

            match NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S") {
                Ok(date_time) => Ok(date_time),
                Err(e) => Err(Error::ExifDateTime((s, e))),
            }
        }
        None => Err(Error::NoExifDateTime),
    }
}

pub fn get_metadata(path: &str, name: &str) -> Result<ImageMetadata, Error> {
    let file_name = disk::get_full_path(path, name);
    let file = File::open(file_name)?;
    let mut reader = BufReader::new(&file);
    let exif = exif::Reader::new().read_from_container(&mut reader)?;

    let orientation = if let Some(field) = exif.get_field(Tag::Orientation, In::PRIMARY) {
        field.value.get_uint(0)
    } else {
        None
    };

    let iso = get_exif_string(&exif, Tag::PhotographicSensitivity);
    let model = get_exif_string(&exif, Tag::Model);
    let exposure_time = get_exif_string(&exif, Tag::ExposureTime);
    let f_number = get_exif_string(&exif, Tag::FNumber);
    let date_time = get_exif_string(&exif, Tag::DateTime);
    let focal_length = get_exif_string(&exif, Tag::FocalLength);

    Ok(ImageMetadata {
        orientation,
        iso,
        model,
        exposure_time,
        f_number,
        date_time,
        focal_length,
    })
}

fn get_exif_string(exif: &Exif, tag: Tag) -> Option<String> {
    exif.get_field(tag, In::PRIMARY).map(|field| {
        field
            .display_value()
            .with_unit(exif)
            .to_string()
            .replace('\"', "")
    })
}
