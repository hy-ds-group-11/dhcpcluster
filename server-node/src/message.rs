use serde::{Deserialize, Serialize};

use crate::Lease;

#[derive(Serialize, Deserialize)]
pub enum Message {
    Join,
    Heartbeat,
    Election,
    Okay,
    Coordinator,
    Add(Lease),
    Update(Lease),
}
