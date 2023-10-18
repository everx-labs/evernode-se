/*
* Copyright 2018-2022 TON DEV SOLUTIONS LTD.
*
* Licensed under the SOFTWARE EVALUATION License (the "License"); you may not use
* this file except in compliance with the License.  You may obtain a copy of the
* License at:
*
* https://www.ton.dev/licenses
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific TON DEV software governing permissions and limitations
* under the License.
*/

use super::TonNodeEngineManager;
use crate::engine::BlockTimeMode;
use crate::NodeError;
use iron::{
    prelude::{IronError, IronResult, Request, Response},
    status,
};
use router::Router;
use std::{sync::Arc, borrow::Cow, str::FromStr};

pub struct ControlApi;

impl ControlApi {
    pub(crate) fn add_route(router: &mut Router, path: String, node: Arc<TonNodeEngineManager>) {
        router.post(
            format!("/{}/:command", path),
            move |req: &mut Request| Self::process_request(req, node.clone()),
            "live_control",
        );
    }

    fn process_request(req: &mut Request, node: Arc<TonNodeEngineManager>) -> Result<Response, IronError> {
        log::info!(target: "node", "Control API request: {}", req.url.path().last().unwrap_or(&""));
        let command = ControlCommand::from_req(req)?;
        let response = match command {
            ControlCommand::IncreaseTime(delta) => {
                node.node().increase_time_delta(delta);
                Response::with(status::Ok)
            }
            ControlCommand::ResetTime => {
                node.node().reset_time_delta();
                Response::with(status::Ok)
            }
            ControlCommand::TimeDelta => {
                Response::with((status::Ok, format!("{}", node.node().time_delta())))
            }
            ControlCommand::TimeMode => {
                Response::with((status::Ok, format!("{}", node.node().time_mode() as u8)))
            }
            ControlCommand::SetTimeMode(mode) => {
                node.node().set_time_mode(mode);
                Response::with(status::Ok)
            }
            ControlCommand::Time => {
                Response::with((status::Ok, format!("{}", node.node().get_next_time())))
            }
            ControlCommand::Reset => {
                node.reset().map_err(server_error)?;
                Response::with(status::Ok)
            }
            ControlCommand::Fork(endpoint, auth, reset) => {
                node.fork(endpoint, auth, reset).map_err(server_error)?;
                Response::with(status::Ok)
            }
            ControlCommand::Unfork(reset) => {
                node.unfork(reset).map_err(server_error)?;
                Response::with(status::Ok)
            }
        };
        Ok(response)

        // log::warn!(target: "node", "Error handling control request");
        // Ok(Response::with((
        //     status::BadRequest,
        //     "Error handling control request",
        // )))
    }
}

enum ControlCommand {
    IncreaseTime(u32),
    ResetTime,
    TimeDelta,
    TimeMode,
    SetTimeMode(BlockTimeMode),
    Time,
    Reset,
    Fork(String, Option<String>, bool),
    Unfork(bool),
}

impl ControlCommand {
    fn from_req(req: &Request) -> IronResult<Self> {
        let url: iron::url::Url = req.url.clone().into();
        if let Some(cmd) = req.extensions.get::<Router>().unwrap().find("command") {
            match cmd.to_lowercase().as_str() {
                "increase-time" => Ok(Self::IncreaseTime(required_parameter(&url, "delta")?)),
                "reset-time" => Ok(Self::ResetTime),
                "time-delta" => Ok(Self::TimeDelta),
                "time-mode" => Ok(Self::TimeMode),
                "set-time-mode" => Ok(Self::SetTimeMode(required_time_mode(&url)?)),
                "time" => Ok(Self::Time),
                "reset" => Ok(Self::Reset),
                "fork" => Ok(Self::Fork(
                    required_parameter(&url, "endpoint")?,
                    optional_parameter(&url, "auth")?,
                    optional_parameter(&url, "resetData")?.unwrap_or_default()
                )),
                "unfork" => Ok(Self::Unfork(
                    optional_parameter(&url, "resetData")?.unwrap_or_default()
                )),
                _ => Err(bad_request(format!(
                    "Unknown live control command \"{}\".",
                    cmd
                ))),
            }
        } else {
            Err(bad_request("Missing live control command".to_string()))
        }
    }
}

fn required_time_mode(url: &iron::url::Url) -> IronResult<BlockTimeMode> {
    match required_parameter::<u32>(url, "mode")? {
        0 => Ok(BlockTimeMode::System),
        1 => Ok(BlockTimeMode::Seq),
        mode => Err(bad_request(format!(
            "Invalid time mode: {}. Expected 0 (System), 1 (Seq)",
            mode
        ))),
    }
}

fn find_parameter<'a>(url: &'a iron::url::Url, name: &str) -> Option<Cow<'a, str>> {
    url.query_pairs().find(|(x, _)| x == name).map(|(_, value)| value)
}

fn required_parameter<R: FromStr + Default>(url: &iron::url::Url, name: &str) -> IronResult<R> {
    optional_parameter(url, name)?
        .ok_or_else(|| bad_request(format!("Missing required parameter {}", name)))
}

fn optional_parameter<R: FromStr>(url: &iron::url::Url, name: &str) -> IronResult<Option<R>> {
    find_parameter(url, name)
        .map(|value| value.parse().map_err(|_| bad_request(format!("Invalid {} value {}", name, value))))
        .transpose()
}

fn bad_request(msg: String) -> IronError {
    iron_error(status::BadRequest, msg)
}

fn server_error(err: NodeError) -> IronError {
    iron_error(status::InternalServerError, err.to_string())
}

fn iron_error(status: status::Status, msg: String) -> IronError {
    let err = NodeError::ApiError(msg.clone());
    IronError::new(err, (status, msg))
}
