use crate::Lease;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Message {
    Join(u32),
    JoinAck(u32),
    Heartbeat,
    Election,
    Okay,
    Coordinator,
    Add(Lease),
    Update(Lease),
}
