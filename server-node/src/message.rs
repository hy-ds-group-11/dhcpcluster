use crate::Lease;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub enum Message {
    Join,
    Heartbeat,
    Election,
    Okay,
    Coordinator,
    Add(Lease),
    Update(Lease),
}
