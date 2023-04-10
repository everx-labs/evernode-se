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

use crate::engine::engine::TonNodeEngine;
use crate::engine::BlockTimeMode;
use crate::NodeError;
use iron::{
    prelude::{IronError, IronResult, Request, Response},
    status,
};
use router::Router;
use std::sync::Arc;

pub struct ControlApi;

impl ControlApi {
    pub(crate) fn add_route(router: &mut Router, path: String, node: Arc<TonNodeEngine>) {
        router.post(
            format!("/{}/:command", path),
            move |req: &mut Request| Self::process_request(req, node.clone()),
            "live_control",
        );
    }

    fn process_request(req: &mut Request, node: Arc<TonNodeEngine>) -> Result<Response, IronError> {
        log::info!(target: "node", "Control API request: {}", req.url.path().last().unwrap_or(&""));
        let command = ControlCommand::from_req(req)?;
        let response = match command {
            ControlCommand::IncreaseTime(delta) => {
                node.increase_time_delta(delta);
                Response::with(status::Ok)
            }
            ControlCommand::ResetTime => {
                node.reset_time_delta();
                Response::with(status::Ok)
            }
            ControlCommand::TimeDelta => {
                Response::with((status::Ok, format!("{}", node.time_delta())))
            }
            ControlCommand::TimeMode => {
                Response::with((status::Ok, format!("{}", node.time_mode() as u8)))
            }
            ControlCommand::SetTimeMode(mode) => {
                node.set_time_mode(mode);
                Response::with(status::Ok)
            }
            ControlCommand::Time => {
                Response::with((status::Ok, format!("{}", node.get_next_time())))
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
}

impl ControlCommand {
    fn from_req(req: &Request) -> IronResult<Self> {
        if let Some(cmd) = req.extensions.get::<Router>().unwrap().find("command") {
            match cmd.to_lowercase().as_str() {
                "increase-time" => Ok(Self::IncreaseTime(required_u32(req, "delta")?)),
                "reset-time" => Ok(Self::ResetTime),
                "time-delta" => Ok(Self::TimeDelta),
                "time-mode" => Ok(Self::TimeMode),
                "set-time-mode" => Ok(Self::SetTimeMode(required_time_mode(req)?)),
                "time" => Ok(Self::Time),
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

fn required_time_mode(req: &Request) -> IronResult<BlockTimeMode> {
    match required_u32(req, "mode")? {
        0 => Ok(BlockTimeMode::System),
        1 => Ok(BlockTimeMode::Seq),
        mode => Err(bad_request(format!(
            "Invalid time mode: {}. Expected 0 (System), 1 (Seq)",
            mode
        ))),
    }
}

fn required_u32(req: &Request, name: &str) -> IronResult<u32> {
    let url: iron::url::Url = req.url.clone().into();
    if let Some((name, value)) = url.query_pairs().find(|(x, _)| x == name) {
        let value = u32::from_str_radix(value.as_ref(), 10)
            .map_err(|_| bad_request(format!("Invalid {} value {}", name, value)))?;
        Ok(value)
    } else {
        Err(bad_request(format!("Missing required parameter {}", name)))
    }
}

fn bad_request(msg: String) -> IronError {
    iron_error(status::BadRequest, msg)
}

fn iron_error(status: status::Status, msg: String) -> IronError {
    let err = NodeError::ApiError(msg.clone());
    IronError::new(err, (status, msg))
}
