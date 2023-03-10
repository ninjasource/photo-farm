#![allow(dead_code)]

use chrono::NaiveDateTime;
use log::error;

use crate::{metadata, ImageNamePair};

pub struct Images {
    path: String,
    inner: Vec<ImageNamePair>,
    index: usize,
}

impl Images {
    pub fn new(path: &str, name: &str, image_file_names: Vec<ImageNamePair>) -> Self {
        let index = Self::get_image_index(name, &image_file_names);
        Self {
            path: path.to_owned(),
            inner: image_file_names,
            index,
        }
    }

    pub fn get_image_index(name: &str, image_file_names: &[ImageNamePair]) -> usize {
        for (i, image_name) in image_file_names.iter().enumerate() {
            if name == image_name.jpg_file_name {
                return i;
            }
        }

        0
    }

    pub fn next(&mut self) {
        if self.index == self.inner.len() - 1 {
            self.index = 0
        } else {
            self.index += 1
        }
    }

    fn set_date_time(&mut self) {
        let path = &self.path.clone();
        let current = self.current_mut();

        if current.date_time.is_none() {
            match metadata::_get_date_time(path, &current.jpg_file_name) {
                Ok(date_time) => current.date_time = Some(date_time),
                Err(e) => {
                    error!("error fetching exif date time {e:?}");
                }
            }
        }
    }

    pub fn next_group(&mut self) {
        // use this until groups are working properly
        self.next();

        /*
        loop {
            let from = self.current().date_time;
            if self.index == self.inner.len() - 1 {
                self.index = 0;
                break;
            } else {
                self.index += 1;
                if self.current().is_starred {
                    break;
                }
                if !self.is_in_group(from) {
                    break;
                }
            }
        }*/
    }

    fn is_in_group(&mut self, from: Option<NaiveDateTime>) -> bool {
        self.set_date_time();

        let current = self.current();
        if from.is_none() || current.date_time.is_none() {
            return false;
        }

        let from = from.unwrap();
        let date_time = current.date_time.unwrap();

        // the difference in timestamp seconds is more than 1 seconds
        if (date_time.timestamp() - from.timestamp()).abs() > 1 {
            return false;
        }

        true
    }

    pub fn prev(&mut self) {
        if self.index == 0 {
            self.index = self.inner.len() - 1
        } else {
            self.index -= 1;
        }
    }

    pub fn prev_group(&mut self) {
        // use this until groups are working properly
        self.prev();

        /*
        loop {
            let from = self.current().date_time;
            if self.index == 0 {
                self.index = self.inner.len() - 1;
                break;
            } else {
                self.index -= 1;
                if self.current().is_starred {
                    break;
                }
                if !self.is_in_group(from) {
                    break;
                }
            }
        }*/
    }

    pub fn next_starred(&mut self) {
        if self.all().iter().any(|x| x.is_starred) {
            loop {
                self.next();

                if self.current().is_starred {
                    break;
                }
            }
        }
    }

    pub fn prev_starred(&mut self) {
        if self.all().iter().any(|x| x.is_starred) {
            loop {
                self.prev();

                if self.current().is_starred {
                    break;
                }
            }
        }
    }

    pub fn current(&self) -> &ImageNamePair {
        &self.inner[self.index]
    }

    pub fn current_mut(&mut self) -> &mut ImageNamePair {
        &mut self.inner[self.index]
    }

    pub fn current_index(&self) -> usize {
        self.index
    }

    pub fn all(&self) -> &Vec<ImageNamePair> {
        &self.inner
    }
}
