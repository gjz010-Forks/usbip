use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::{net::TcpStream, task::JoinSet};

mod common;
use common::*;
use usbip::usbip_protocol::{USBIP_CMD_SUBMIT, UsbIpCommand, UsbIpHeaderBasic, UsbIpResponse};
use usbip::*;

const SINGLE_DEVICE_BUSID: &str = "0-0-0";

fn new_server_with_single_device() -> UsbIpServer {
    UsbIpServer::new_simulated(vec![UsbDevice::new(0).with_interface(
        ClassCode::CDC as u8,
        cdc::CDC_ACM_SUBCLASS,
        0x00,
        Some("Test CDC ACM"),
        cdc::UsbCdcAcmHandler::endpoints(),
        Arc::new(Mutex::new(
            Box::new(cdc::UsbCdcAcmHandler::new()) as Box<dyn UsbInterfaceHandler + Send>
        )),
    )])
}

fn op_req_import(busid: &str) -> Vec<u8> {
    let mut busid = busid.to_string().as_bytes().to_vec();
    busid.resize(32, 0);
    UsbIpCommand::OpReqImport {
        status: 0,
        busid: busid.try_into().unwrap(),
    }
    .to_bytes()
}

async fn attach_device(connection: &mut TcpStream, busid: &str) -> u32 {
    let req = op_req_import(busid);
    connection.write_all(req.as_slice()).await.unwrap();
    connection.read_u32().await.unwrap();
    let result = connection.read_u32().await.unwrap();
    if result == 0 {
        connection.read_exact(&mut vec![0; 0x138]).await.unwrap();
    }
    result
}

#[tokio::test]
async fn req_empty_devlist() {
    setup_test_logger();
    let server = UsbIpServer::new_simulated(vec![]);
    let req = UsbIpCommand::OpReqDevlist { status: 0 };

    let mut mock_socket = MockSocket::new(req.to_bytes());
    handler(&mut mock_socket, Arc::new(server)).await.ok();

    assert_eq!(
        mock_socket.output,
        UsbIpResponse::op_rep_devlist(&[]).to_bytes(),
    );
}

#[tokio::test]
async fn req_sample_devlist() {
    setup_test_logger();
    let server = new_server_with_single_device();
    let req = UsbIpCommand::OpReqDevlist { status: 0 };

    let mut mock_socket = MockSocket::new(req.to_bytes());
    handler(&mut mock_socket, Arc::new(server)).await.ok();

    // OP_REP_DEVLIST
    // header: 0xC
    // device: 0x138
    // interface: 4 * 0x1
    assert_eq!(mock_socket.output.len(), 0xC + 0x138 + 4);
}

#[tokio::test]
async fn req_import() {
    setup_test_logger();
    let server = new_server_with_single_device();

    // OP_REQ_IMPORT
    let req = op_req_import(SINGLE_DEVICE_BUSID);
    let mut mock_socket = MockSocket::new(req);
    handler(&mut mock_socket, Arc::new(server)).await.ok();
    // OP_REQ_IMPORT
    assert_eq!(mock_socket.output.len(), 0x140);
}

#[tokio::test]
async fn add_and_remove_10_devices() {
    setup_test_logger();
    let server_ = Arc::new(UsbIpServer::new_simulated(vec![]));
    let addr = get_free_address().await;
    tokio::spawn(server(addr, server_.clone()));

    let mut join_set = JoinSet::new();
    let devices = (0..10).map(UsbDevice::new).collect::<Vec<_>>();

    for device in devices.iter() {
        let new_server = server_.clone();
        let new_device = device.clone();
        join_set.spawn(async move {
            new_server.add_device(new_device).await;
        });
    }

    for device in devices.iter() {
        let new_server = server_.clone();
        let new_device = device.clone();
        join_set.spawn(async move {
            new_server.remove_device(&new_device.bus_id).await.unwrap();
        });
    }

    while join_set.join_next().await.is_some() {}

    let device_len = server_.clone().available_devices().await.len();

    assert_eq!(device_len, 0);
}

#[tokio::test]
async fn send_usb_traffic_while_adding_and_removing_devices() {
    setup_test_logger();
    let server_ = Arc::new(new_server_with_single_device());

    let addr = get_free_address().await;
    tokio::spawn(server(addr, server_.clone()));

    let cmd_loop_handle = tokio::spawn(async move {
        let mut connection = poll_connect(addr).await;
        let result = attach_device(&mut connection, SINGLE_DEVICE_BUSID).await;
        assert_eq!(result, 0);

        let cdc_loopback_bulk_cmd = UsbIpCommand::UsbIpCmdSubmit {
            header: usbip_protocol::UsbIpHeaderBasic {
                command: USBIP_CMD_SUBMIT.into(),
                seqnum: 1,
                devid: 0,
                direction: 0, // OUT
                ep: 2,
            },
            transfer_flags: 0,
            transfer_buffer_length: 8,
            start_frame: 0,
            number_of_packets: 0,
            interval: 0,
            setup: [0; 8],
            data: vec![1, 2, 3, 4, 5, 6, 7, 8],
            iso_packet_descriptor: vec![],
        };

        loop {
            connection
                .write_all(cdc_loopback_bulk_cmd.to_bytes().as_slice())
                .await
                .unwrap();
            let mut result = vec![0; 4 * 12];
            connection.read_exact(&mut result).await.unwrap();
        }
    });

    let add_and_remove_device_handle = tokio::spawn(async move {
        let mut join_set = JoinSet::new();
        let devices = (1..4).map(UsbDevice::new).collect::<Vec<_>>();

        loop {
            for device in devices.iter() {
                let new_server = server_.clone();
                let new_device = device.clone();
                join_set.spawn(async move {
                    new_server.add_device(new_device).await;
                });
            }

            for device in devices.iter() {
                let new_server = server_.clone();
                let new_device = device.clone();
                join_set.spawn(async move {
                    new_server.remove_device(&new_device.bus_id).await.unwrap();
                });
            }
            while join_set.join_next().await.is_some() {}
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    cmd_loop_handle.abort();
    add_and_remove_device_handle.abort();
}

#[tokio::test]
async fn only_single_connection_allowed_to_device() {
    setup_test_logger();
    let server_ = Arc::new(new_server_with_single_device());

    let addr = get_free_address().await;
    tokio::spawn(server(addr, server_.clone()));

    let mut first_connection = poll_connect(addr).await;
    let mut second_connection = TcpStream::connect(addr).await.unwrap();

    let result = attach_device(&mut first_connection, SINGLE_DEVICE_BUSID).await;
    assert_eq!(result, 0);

    let result = attach_device(&mut second_connection, SINGLE_DEVICE_BUSID).await;
    assert_eq!(result, 1);
}

#[tokio::test]
async fn device_gets_released_on_closed_socket() {
    setup_test_logger();
    let server_ = Arc::new(new_server_with_single_device());

    let addr = get_free_address().await;
    tokio::spawn(server(addr, server_.clone()));

    let mut connection = poll_connect(addr).await;
    let result = attach_device(&mut connection, SINGLE_DEVICE_BUSID).await;
    assert_eq!(result, 0);

    std::mem::drop(connection);

    let mut connection = TcpStream::connect(addr).await.unwrap();
    let result = attach_device(&mut connection, SINGLE_DEVICE_BUSID).await;
    assert_eq!(result, 0);
}

#[tokio::test]
async fn req_import_get_device_desc() {
    setup_test_logger();
    let server = new_server_with_single_device();

    let mut req = op_req_import(SINGLE_DEVICE_BUSID);
    req.extend(
        UsbIpCommand::UsbIpCmdSubmit {
            header: UsbIpHeaderBasic {
                command: USBIP_CMD_SUBMIT.into(),
                seqnum: 1,
                devid: 0,
                direction: 1, // IN
                ep: 0,
            },
            transfer_flags: 0,
            transfer_buffer_length: 0,
            start_frame: 0,
            number_of_packets: 0,
            interval: 0,
            // GetDescriptor to Device
            setup: [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x40, 0x00],
            data: vec![],
            iso_packet_descriptor: vec![],
        }
        .to_bytes(),
    );

    let mut mock_socket = MockSocket::new(req);
    handler(&mut mock_socket, Arc::new(server)).await.ok();
    // OP_REQ_IMPORT + USBIP_CMD_SUBMIT + Device Descriptor
    assert_eq!(mock_socket.output.len(), 0x140 + 0x30 + 0x12);
}
