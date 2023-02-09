use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::{Error, ImageNamePair};

pub fn get_file_names(path: &str) -> Result<Vec<ImageNamePair>, Error> {
    let jpegs = get_image_file_names(path)?;
    let others = get_other_file_names(path)?;

    // build a lookup of all file names that are not jpegs
    let mut lookup: HashMap<String, Vec<String>> = HashMap::new();
    for other in others {
        let name = get_lowercase_name_without_extension(&other);
        lookup
            .entry(name)
            .and_modify(|files| files.push(other.to_owned()))
            .or_insert(vec![other]);
    }

    // attempt to match other files with jpeg files by name
    // in the unlikely event that we encounter a jpg and jpeg with the same name
    // only one of the two jpg files will have other_files associated with it
    let items: Vec<ImageNamePair> = jpegs
        .into_iter()
        .map(|jpeg| {
            let name = get_lowercase_name_without_extension(&jpeg);
            match lookup.remove(&name) {
                Some(files) => ImageNamePair {
                    jpg_file_name: jpeg,
                    other_file_names: files,
                },
                None => ImageNamePair {
                    jpg_file_name: jpeg,
                    other_file_names: vec![],
                },
            }
        })
        .collect();

    Ok(items)
}

pub fn get_full_path(path: &str, name: &str) -> String {
    PathBuf::from_str(path)
        .expect(&format!("not a falid path: {path}"))
        .join(name)
        .to_str()
        .expect("full path is empty")
        .to_owned()
}

fn get_lowercase_name_without_extension(name: &str) -> String {
    let name = Path::new(name).file_stem().expect("name is not a file");
    let name = name.to_str().expect("file name is empty");
    name.to_lowercase()
}

fn get_image_file_names(path: &str) -> Result<Vec<String>, Error> {
    let directory = std::fs::read_dir(path)?;
    let mut files: Vec<String> = directory
        .filter_map(|x| {
            let path = x.expect("cannot read directory");
            let path = path.file_name();
            let path = path.to_str().expect("image file name is empty");
            if path.to_lowercase().ends_with(".jpg") || path.to_lowercase().ends_with(".jpeg") {
                Some(path.to_owned())
            } else {
                None
            }
        })
        .collect();
    files.sort();

    Ok(files)
}

fn get_other_file_names(path: &str) -> Result<Vec<String>, Error> {
    let directory = std::fs::read_dir(path)?;
    let mut files: Vec<String> = directory
        .filter_map(|x| {
            let path = x.expect("cannot read directory");
            let path = path.file_name();
            let path = path.to_str().expect("image file name is empty");
            if !path.to_lowercase().ends_with(".jpg") && !path.to_lowercase().ends_with(".jpeg") {
                Some(path.to_owned())
            } else {
                None
            }
        })
        .collect();
    files.sort();

    Ok(files)
}

pub fn export(path: &str, image_pairs: &Vec<&ImageNamePair>) -> Result<(), Error> {
    let mut to_path = PathBuf::from_str(path).expect(&format!("not a falid path: {path}"));
    to_path.push("export");
    let to_path = to_path.to_str().expect("path is empty");
    fs::create_dir_all(to_path)?;

    for pair in image_pairs {
        copy_file(path, to_path, &pair.jpg_file_name)?;
        for other in pair.other_file_names.iter() {
            copy_file(path, to_path, other)?;
        }
    }

    Ok(())
}

fn copy_file(from_path: &str, to_path: &str, name: &str) -> Result<(), Error> {
    let from_file = get_full_path(from_path, name);
    let to_file = get_full_path(to_path, name);
    fs::copy(from_file, to_file)?;
    Ok(())
}
