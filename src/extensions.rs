use crate::error::GAErrors::ParseError;
use crate::user::User;

use core::result::Result as CoreResult;
use std::fs;
use std::io::Result as IoResult;
use std::path::Path;

use actix_web::HttpRequest;
use anyhow::{Context, Error, Result};
use log::warn;
use sqlx::{Transaction, Postgres};

/// Parses "key=value" into a key value tuple
pub(crate) fn parse_key_value(input: &str) -> Result<(&str, &str)> {
    let mut split = input.splitn(2, "=");
    let key = split.next().ok_or(ParseError("key values", input.to_owned()))?;
    let value = split.next().ok_or(ParseError("key values", input.to_owned()))?;

    Ok((key, value))
}

pub(crate) fn get_header<'a>(request: &'a HttpRequest, header: &'a str) -> Option<&'a str> {
    request.headers().get(header)?.to_str().ok()
}

pub(crate) async fn get_user_by_identity(identity: Option<String>, transaction: &mut Transaction<'_, Postgres>) -> Option<User> {
    match identity {
        Some(id_str) => {
            let mut split = id_str.splitn(2, '$');

            let id = split.next().unwrap_or_else(|| {
                warn!("Unable to parse id from identity string `{}`", id_str);
                "-1"
            }).parse::<i32>().unwrap_or(-1);

            let session = split.next().unwrap_or_else(|| {
                warn!("Unable to parse session from identity string `{}`", id_str);
                "unknown"
            });

            sqlx::query_as::<_, User>("select * from users where id = $1 and session = $2 limit 1")
                .bind(&id)
                .bind(session)
                .fetch_one(transaction)
                .await
                .ok()
        }
        None => None
    }
}

/// Checks if the character is alphanumeric (`a-z, 0-9`), a dash (`-`) or a underscore (`_`)
#[inline]
pub(crate) fn is_identifier(c: &char) -> bool {
    c.is_ascii_alphanumeric() || c == &'-' || c == &'_'
}

/// Checks for illegal file and directory names on Windows.
/// This function assumes that the input has already been checked with [`is_identifier`][0].
///
/// [0]: crate::extensions::is_identifier
pub(crate) async fn is_fs_legal(input: &String) -> bool {
    let mut legal = input != "CON";
    legal &= input != "PRN";
    legal &= input != "AUX";
    legal &= input != "NUL";
    legal &= input != "LST";

    for i in 0..=9 {
        legal &= input != &format!("COM{}", i);
        legal &= input != &format!("LPT{}", i);
    }

    legal
}

/// Flattens `std::io::Result<std::result::Result<O, E>>` into `anyhow::Result<O>`
pub(crate) fn flatten_io_result<O, E: Into<Error>>(result: IoResult<CoreResult<O, E>>) -> Result<O> {
    match result {
        Ok(Ok(ok)) => Ok(ok),
        Ok(Err(err)) => Err(err.into()),
        Err(err) => Err(err.into())
    }
}

/// Flattens `std::result::Result<std::result::Result<O, E>, E>` into `anyhow::Result<O>`
pub(crate) fn flatten_result<O, E: Into<Error>>(result: CoreResult<CoreResult<O, E>, E>) -> Result<O> {
    match result {
        Ok(Ok(ok)) => Ok(ok),
        Ok(Err(err)) => Err(err.into()),
        Err(err) => Err(err.into())
    }
}

pub(crate) fn create_dir_if_not_exists(path: &Path) -> Result<()> {
    if !path.is_dir() {
        // Check if path is a file and not a directory
        if path.exists() {
            fs::remove_file(path).context("Unable to delete file")?;
        }

        return fs::create_dir_all(path).context("Unable to create directory");
    }

    Ok(())
}
