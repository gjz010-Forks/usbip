//! A library for running a USB/IP server

use log::*;
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
//use rusb::*;
use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::io::Result;
use std::sync::{Arc, Mutex};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

mod consts;
mod device;
mod devices;
mod endpoint;
mod interface;
mod setup;
pub mod usbip_protocol;
mod util;
pub use consts::*;
pub use device::*;
#[cfg(feature = "rusb")]
pub use devices::host::*;
pub use devices::{cdc, hid};
pub use endpoint::*;
pub use interface::*;
pub use setup::*;
pub use util::*;
mod usbip_server;
pub use usbip_server::{
    UsbIpServer,
    server::{handler, server},
};
