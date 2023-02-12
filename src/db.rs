use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, Mutex},
};

use log::info;
use speedy2d::dimen::UVec2;
use sqlite::{Connection, State, Value};

use crate::disk;
use crate::Error;

const DB_TABLE_PHOTOS: &str = "photos";
const DB_COL_NAME: &str = "name";
const DB_COL_X_RES: &str = "x_res";
const DB_COL_Y_RES: &str = "y_res";
const DB_COL_RESIZED: &str = "resized";
const DB_COL_IS_STARRED: &str = "is_starred";

pub fn photo_exists(
    name: &str,
    size: UVec2,
    connection: Arc<Mutex<Connection>>,
) -> Result<bool, Error> {
    let connection = connection.lock().unwrap();

    let query = format!("SELECT 1 FROM {DB_TABLE_PHOTOS} WHERE {DB_COL_NAME} = :{DB_COL_NAME} AND {DB_COL_X_RES} = :{DB_COL_X_RES} AND {DB_COL_Y_RES} = :{DB_COL_Y_RES};");
    let mut statement = connection.prepare(query)?;
    let x = size.x as i64;
    let y = size.y as i64;

    statement.bind::<&[(_, Value)]>(
        &[
            (format!(":{DB_COL_NAME}").as_str(), name.into()),
            (format!(":{DB_COL_X_RES}").as_str(), x.into()),
            (format!(":{DB_COL_Y_RES}").as_str(), y.into()),
        ][..],
    )?;

    match statement.next()? {
        State::Row => Ok(true),
        State::Done => Ok(false),
    }
}

pub fn try_get_image_from_db(
    name: &str,
    size: UVec2,
    connection: Arc<Mutex<Connection>>,
) -> Result<Option<Vec<u8>>, Error> {
    let connection = connection.lock().unwrap();

    // even if there is an entry in the db there may not yet be a resized image
    let query = format!(
        "SELECT {DB_COL_RESIZED} FROM {DB_TABLE_PHOTOS} WHERE {DB_COL_NAME} = :{DB_COL_NAME} AND {DB_COL_X_RES} = :{DB_COL_X_RES} AND {DB_COL_Y_RES} = :{DB_COL_Y_RES} AND NOT {DB_COL_RESIZED} IS NULL;"
    );

    let mut statement = connection.prepare(query)?;
    let x = size.x as i64;
    let y = size.y as i64;

    statement.bind::<&[(_, Value)]>(
        &[
            (format!(":{DB_COL_NAME}").as_str(), name.into()),
            (format!(":{DB_COL_X_RES}").as_str(), x.into()),
            (format!(":{DB_COL_Y_RES}").as_str(), y.into()),
        ][..],
    )?;

    match statement.next()? {
        State::Row => {
            let resized = statement.read::<Vec<u8>, _>(DB_COL_RESIZED)?;
            Ok(Some(resized))
        }
        State::Done => Ok(None),
    }
}

pub fn insert_image(
    name: &str,
    size: UVec2,
    resized: &[u8],
    connection: Arc<Mutex<Connection>>,
) -> Result<(), Error> {
    let connection = connection.lock().unwrap();

    let query = format!(
        "INSERT INTO {DB_TABLE_PHOTOS} VALUES (:{DB_COL_NAME}, :{DB_COL_X_RES}, :{DB_COL_Y_RES}, :{DB_COL_RESIZED}, :{DB_COL_IS_STARRED});"
    );
    let mut statement = connection.prepare(query)?;
    let x = size.x as i64;
    let y = size.y as i64;
    let is_starred: i64 = 0;

    statement.bind::<&[(_, Value)]>(
        &[
            (format!(":{DB_COL_NAME}").as_str(), name.into()),
            (format!(":{DB_COL_X_RES}").as_str(), x.into()),
            (format!(":{DB_COL_Y_RES}").as_str(), y.into()),
            (format!(":{DB_COL_RESIZED}").as_str(), resized.into()),
            (format!(":{DB_COL_IS_STARRED}").as_str(), is_starred.into()),
        ][..],
    )?;

    statement.next()?;
    Ok(())
}

pub fn update_image_is_starred(
    name: &str,
    is_starred: bool,
    connection: Arc<Mutex<Connection>>,
) -> Result<(), Error> {
    let connection = connection.lock().unwrap();
    let query = format!("UPDATE {DB_TABLE_PHOTOS} SET {DB_COL_IS_STARRED} = :{DB_COL_IS_STARRED} WHERE {DB_COL_NAME} = :{DB_COL_NAME};");
    let mut statement = connection.prepare(query)?;
    let is_starred = is_starred as i64;

    statement.bind::<&[(_, Value)]>(
        &[
            (format!(":{DB_COL_IS_STARRED}").as_str(), is_starred.into()),
            (format!(":{DB_COL_NAME}").as_str(), name.into()),
        ][..],
    )?;

    statement.next()?;
    Ok(())
}

pub fn get_starred_image_names(
    connection: Arc<Mutex<Connection>>,
) -> Result<HashSet<String>, Error> {
    let connection = connection.lock().unwrap();

    let query =
        format!("SELECT {DB_COL_NAME} FROM {DB_TABLE_PHOTOS} WHERE {DB_COL_IS_STARRED} = TRUE;");
    let mut statement = connection.prepare(query)?;
    let mut names = HashSet::new();

    while let State::Row = statement.next()? {
        let name = statement.read::<String, _>(DB_COL_NAME)?;
        names.insert(name);
    }

    Ok(names)
}

pub fn get_or_create_db(path: &str) -> Result<Connection, Error> {
    // a sqlite3 database
    let db_file_name = disk::get_full_path(path, "thumbnails.db");
    info!("Opening database: {db_file_name}");
    if Path::new(&db_file_name).exists() {
        Ok(sqlite::open(&db_file_name)?)
    } else {
        let connection = sqlite::open(&db_file_name)?;
        let query = format!("CREATE TABLE {DB_TABLE_PHOTOS} ({DB_COL_NAME} TEXT, {DB_COL_X_RES} INTEGER, {DB_COL_Y_RES} INTEGER, {DB_COL_RESIZED} BLOB, {DB_COL_IS_STARRED} INTEGER);");
        connection.execute(query)?;
        Ok(connection)
    }
}
