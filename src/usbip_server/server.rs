use std::{net::SocketAddr, sync::Arc};

use crate::{
    SetupPacket, UsbIpServer,
    usbip_protocol::{USBIP_RET_SUBMIT, USBIP_RET_UNLINK, UsbIpCommand, UsbIpResponse},
};
use log::*;
use std::io::{ErrorKind, Result};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub async fn handler<T: AsyncReadExt + AsyncWriteExt + Unpin>(
    mut socket: &mut T,
    server: Arc<UsbIpServer>,
) -> Result<()> {
    let mut current_import_device_id: Option<String> = None;
    loop {
        let command = UsbIpCommand::read_from_socket(&mut socket).await;
        if let Err(err) = command {
            if let Some(dev_id) = current_import_device_id {
                let mut used_devices = server.used_devices.write().await;
                let mut available_devices = server.available_devices.write().await;
                match used_devices.remove(&dev_id) {
                    Some(dev) => available_devices.push(dev),
                    None => unreachable!(),
                }
            }

            if err.kind() == ErrorKind::UnexpectedEof {
                info!("Remote closed the connection");
                return Ok(());
            } else {
                return Err(err);
            }
        }

        let used_devices = server.used_devices.read().await;
        let mut current_import_device = current_import_device_id
            .clone()
            .and_then(|ref id| used_devices.get(id));

        match command.unwrap() {
            UsbIpCommand::OpReqDevlist { .. } => {
                trace!("Got OP_REQ_DEVLIST");
                let devices = server.available_devices.read().await;

                // OP_REP_DEVLIST
                UsbIpResponse::op_rep_devlist(&devices)
                    .write_to_socket(socket)
                    .await?;
                trace!("Sent OP_REP_DEVLIST");
            }
            UsbIpCommand::OpReqImport { busid, .. } => {
                trace!("Got OP_REQ_IMPORT");

                current_import_device_id = None;
                current_import_device = None;
                std::mem::drop(used_devices);

                let mut used_devices = server.used_devices.write().await;
                let mut available_devices = server.available_devices.write().await;
                let busid_compare =
                    &busid[..busid.iter().position(|&x| x == 0).unwrap_or(busid.len())];
                for (i, dev) in available_devices.iter().enumerate() {
                    if busid_compare == dev.bus_id.as_bytes() {
                        let dev = available_devices.remove(i);
                        let dev_id = dev.bus_id.clone();
                        used_devices.insert(dev.bus_id.clone(), dev);
                        current_import_device_id = dev_id.clone().into();
                        current_import_device = Some(used_devices.get(&dev_id).unwrap());
                        break;
                    }
                }

                let res = if let Some(dev) = current_import_device {
                    UsbIpResponse::op_rep_import_success(dev)
                } else {
                    UsbIpResponse::op_rep_import_fail()
                };
                res.write_to_socket(socket).await?;
                trace!("Sent OP_REP_IMPORT");
            }
            UsbIpCommand::UsbIpCmdSubmit {
                mut header,
                transfer_buffer_length,
                setup,
                data,
                ..
            } => {
                trace!("Got USBIP_CMD_SUBMIT");
                let device = current_import_device.unwrap();

                let out = header.direction == 0;
                let real_ep = if out { header.ep } else { header.ep | 0x80 };

                header.command = USBIP_RET_SUBMIT.into();

                let res = match device.find_ep(real_ep as u8) {
                    None => {
                        warn!("Endpoint {real_ep:02x?} not found");
                        UsbIpResponse::usbip_ret_submit_fail(&header)
                    }
                    Some((ep, intf)) => {
                        trace!("->Endpoint {ep:02x?}");
                        trace!("->Setup {setup:02x?}");
                        trace!("->Request {data:02x?}");
                        let resp = device
                            .handle_urb(
                                ep,
                                intf,
                                transfer_buffer_length,
                                SetupPacket::parse(&setup),
                                &data,
                            )
                            .await;

                        match resp {
                            Ok(resp) => {
                                if out {
                                    trace!("<-Wrote {}", data.len());
                                } else {
                                    trace!("<-Resp {resp:02x?}");
                                }
                                UsbIpResponse::usbip_ret_submit_success(&header, 0, 0, resp, vec![])
                            }
                            Err(err) => {
                                warn!("Error handling URB: {err}");
                                UsbIpResponse::usbip_ret_submit_fail(&header)
                            }
                        }
                    }
                };
                res.write_to_socket(socket).await?;
                trace!("Sent USBIP_RET_SUBMIT");
            }
            UsbIpCommand::UsbIpCmdUnlink {
                mut header,
                unlink_seqnum,
            } => {
                trace!("Got USBIP_CMD_UNLINK for {unlink_seqnum:10x?}");

                header.command = USBIP_RET_UNLINK.into();

                let res = UsbIpResponse::usbip_ret_unlink_success(&header);
                res.write_to_socket(socket).await?;
                trace!("Sent USBIP_RET_UNLINK");
            }
        }
    }
}

/// Spawn a USB/IP server at `addr` using [TcpListener]
pub async fn server(addr: SocketAddr, server: Arc<UsbIpServer>) {
    let listener = TcpListener::bind(addr).await.expect("bind to addr");

    let server = async move {
        loop {
            match listener.accept().await {
                Ok((mut socket, _addr)) => {
                    info!("Got connection from {:?}", socket.peer_addr());
                    let new_server = server.clone();
                    tokio::spawn(async move {
                        let res = handler(&mut socket, new_server).await;
                        info!("Handler ended with {res:?}");
                    });
                }
                Err(err) => {
                    warn!("Got error {err:?}");
                }
            }
        }
    };

    server.await
}
