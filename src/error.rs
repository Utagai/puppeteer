use std::io::Cursor;

use rocket::{
    http::ContentType,
    response::{self, Responder},
    serde::json,
    Response,
};
use serde::{Deserialize, Serialize};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("puppet with id {0} not found")]
    PuppetNotFound(i32),
    #[error("io error")]
    IOError(#[from] std::io::Error),
}

#[derive(Serialize, Deserialize)]
struct ErrorJSONResp {
    err: String,
}

impl<'r> Responder<'r, 'r> for Error {
    fn respond_to(self, request: &'r rocket::Request<'_>) -> rocket::response::Result<'r> {
        let err_resp = ErrorJSONResp {
            err: format!("{:?}", self),
        };
        match json::to_string(&err_resp) {
            Ok(err_json) => Response::build()
                .header(ContentType::JSON)
                .sized_body(err_json.len(), Cursor::new(err_json))
                .ok(),
            Err(err) => response::Debug(err).respond_to(request),
        }
    }
}
