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

use crate::engine::{LiveControl, LiveControlReceiver};
use iron::prelude::*;
use iron::status;
use router::Router;
use std::sync::{Arc, Mutex};
use crate::{NodeError, NodeResult};

pub struct ControlApi {
    path: String,
    router: Arc<Mutex<Option<Router>>>,
}

impl ControlApi {
    pub fn new(path: String, router: Arc<Mutex<Option<Router>>>) -> NodeResult<Self> {
        Ok(Self { path, router })
    }

    fn process_request(
        req: &mut Request,
        control: &Box<dyn LiveControl>,
    ) -> Result<Response, IronError> {
        log::info!(target: "node", "Control API request: {}", req.url.path().last().unwrap_or(&""));
        let command = ControlCommand::from_req(req)?;
        match command {
            ControlCommand::IncreaseTime(delta) => {
                control.increase_time(delta).map_err(|err| {
                    internal_server_error(format!("Increase time failed: {}", err))
                })?;
            }
            ControlCommand::ResetTime => {
                control
                    .reset_time()
                    .map_err(|err| internal_server_error(format!("Reset time failed: {}", err)))?;
            }
        }
        return Ok(Response::with(status::Ok));

        // log::warn!(target: "node", "Error handling control request");
        // Ok(Response::with((
        //     status::BadRequest,
        //     "Error handling control request",
        // )))
    }
}

impl LiveControlReceiver for ControlApi {
    fn run(&self, control: Box<dyn LiveControl>) -> NodeResult<()> {
        if let Some(ref mut router) = *self.router.lock().unwrap() {
            router.post(
                format!("/{}/:command", self.path),
                move |req: &mut Request| Self::process_request(req, &control),
                "live_control",
            );
        }
        Ok(())
    }
}

enum ControlCommand {
    IncreaseTime(u32),
    ResetTime,
}

impl ControlCommand {
    fn from_req(req: &Request) -> IronResult<Self> {
        if let Some(cmd) = req.extensions.get::<Router>().unwrap().find("command") {
            match cmd.to_lowercase().as_str() {
                "increase-time" => Ok(Self::IncreaseTime(required_u32(req, "delta")?)),
                "reset-time" => Ok(Self::ResetTime),
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

fn internal_server_error(msg: String) -> IronError {
    iron_error(status::InternalServerError, msg)
}

fn iron_error(status: status::Status, msg: String) -> IronError {
    let err = NodeError::ApiError(msg.clone());
    IronError::new(err, (status, msg))
}
