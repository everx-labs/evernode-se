use super::*;
use iron::prelude::*;
use iron::status;
use router::Router;
use std::sync::Mutex;
use ton_node::node_engine::{LiveControl, LiveControlReceiver};

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
        log::info!(target: "node", "Control API: request got!");
        log::info!(target: "node", "{:?}", req.url.path());
        let command = ControlCommand::from_req(req)?;
        match command {
            ControlCommand::IncreaseTime(delta) => {
                control.increase_time(delta).map_err(|err| {
                    internal_server_error(format!("Increase time failed: {}", err))
                })?;
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
}

impl ControlCommand {
    fn from_req(req: &Request) -> IronResult<Self> {
        if let Some(cmd) = req.extensions.get::<Router>().unwrap().find("command") {
            match cmd.to_lowercase().as_str() {
                "increase-time" => Ok(Self::IncreaseTime(required_u32(req, "delta")?)),
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
