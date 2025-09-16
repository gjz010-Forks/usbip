use std::sync::{Arc, Mutex};

use log::*;
use rusb::{Device, DeviceHandle, GlobalContext};
use tokio::sync::RwLock;

use crate::{
    EndpointAttributes, RusbUsbHostDeviceHandler, RusbUsbHostInterfaceHandler, UsbDevice,
    UsbEndpoint, UsbInterface, UsbInterfaceHandler, UsbIpServer,
};

impl UsbIpServer {
    /// Create a [UsbIpServer] with Vec<[rusb::DeviceHandle]> for sharing host devices
    pub fn with_rusb_device_handles(
        device_handles: Vec<DeviceHandle<GlobalContext>>,
    ) -> Vec<UsbDevice> {
        let mut devices = vec![];
        for open_device in device_handles {
            let dev = open_device.device();
            let desc = match dev.device_descriptor() {
                Ok(desc) => desc,
                Err(err) => {
                    warn!(
                        "Impossible to get device descriptor for {dev:?}: {err}, ignoring device",
                    );
                    continue;
                }
            };
            let cfg = match dev.active_config_descriptor() {
                Ok(desc) => desc,
                Err(err) => {
                    warn!(
                        "Impossible to get config descriptor for {dev:?}: {err}, ignoring device",
                    );
                    continue;
                }
            };

            let handle = Arc::new(Mutex::new(open_device));
            let mut interfaces = vec![];
            handle
                .lock()
                .unwrap()
                .set_auto_detach_kernel_driver(true)
                .ok();
            for intf in cfg.interfaces() {
                // ignore alternate settings
                let intf_desc = intf.descriptors().next().unwrap();
                handle
                    .lock()
                    .unwrap()
                    .set_auto_detach_kernel_driver(true)
                    .ok();
                let mut endpoints = vec![];

                for ep_desc in intf_desc.endpoint_descriptors() {
                    endpoints.push(UsbEndpoint {
                        address: ep_desc.address(),
                        attributes: ep_desc.transfer_type() as u8,
                        max_packet_size: ep_desc.max_packet_size(),
                        interval: ep_desc.interval(),
                    });
                }

                let handler = Arc::new(Mutex::new(Box::new(RusbUsbHostInterfaceHandler::new(
                    handle.clone(),
                ))
                    as Box<dyn UsbInterfaceHandler + Send>));
                interfaces.push(UsbInterface {
                    interface_class: intf_desc.class_code(),
                    interface_subclass: intf_desc.sub_class_code(),
                    interface_protocol: intf_desc.protocol_code(),
                    endpoints,
                    string_interface: intf_desc.description_string_index().unwrap_or(0),
                    class_specific_descriptor: Vec::from(intf_desc.extra()),
                    handler,
                });
            }
            let mut device = UsbDevice {
                path: format!(
                    "/sys/bus/{}/{}/{}",
                    dev.bus_number(),
                    dev.address(),
                    dev.port_number()
                ),
                bus_id: format!(
                    "{}-{}-{}",
                    dev.bus_number(),
                    dev.address(),
                    dev.port_number()
                ),
                bus_num: dev.bus_number() as u32,
                dev_num: dev.port_number() as u32,
                speed: dev.speed() as u32,
                vendor_id: desc.vendor_id(),
                product_id: desc.product_id(),
                device_class: desc.class_code(),
                device_subclass: desc.sub_class_code(),
                device_protocol: desc.protocol_code(),
                device_bcd: desc.device_version().into(),
                configuration_value: cfg.number(),
                num_configurations: desc.num_configurations(),
                ep0_in: UsbEndpoint {
                    address: 0x80,
                    attributes: EndpointAttributes::Control as u8,
                    max_packet_size: desc.max_packet_size() as u16,
                    interval: 0,
                },
                ep0_out: UsbEndpoint {
                    address: 0x00,
                    attributes: EndpointAttributes::Control as u8,
                    max_packet_size: desc.max_packet_size() as u16,
                    interval: 0,
                },
                interfaces,
                device_handler: Some(Arc::new(Mutex::new(Box::new(
                    RusbUsbHostDeviceHandler::new(handle.clone()),
                )))),
                usb_version: desc.usb_version().into(),
                ..UsbDevice::default()
            };

            // set strings
            if let Some(index) = desc.manufacturer_string_index() {
                device.string_manufacturer = device.new_string(
                    &handle
                        .lock()
                        .unwrap()
                        .read_string_descriptor_ascii(index)
                        .unwrap(),
                )
            }
            if let Some(index) = desc.product_string_index() {
                device.string_product = device.new_string(
                    &handle
                        .lock()
                        .unwrap()
                        .read_string_descriptor_ascii(index)
                        .unwrap(),
                )
            }
            if let Some(index) = desc.serial_number_string_index() {
                device.string_serial = device.new_string(
                    &handle
                        .lock()
                        .unwrap()
                        .read_string_descriptor_ascii(index)
                        .unwrap(),
                )
            }
            devices.push(device);
        }
        devices
    }

    fn with_rusb_devices(device_list: Vec<Device<GlobalContext>>) -> Vec<UsbDevice> {
        let mut device_handles = vec![];

        for dev in device_list {
            let open_device = match dev.open() {
                Ok(dev) => dev,
                Err(err) => {
                    warn!("Impossible to share {dev:?}: {err}, ignoring device");
                    continue;
                }
            };
            device_handles.push(open_device);
        }
        Self::with_rusb_device_handles(device_handles)
    }

    /// Create a [UsbIpServer] exposing devices in the host, and redirect all USB transfers to them using libusb
    pub fn new_from_host() -> Self {
        Self::new_from_host_with_filter(|_| true)
    }

    /// Create a [UsbIpServer] exposing filtered devices in the host, and redirect all USB transfers to them using libusb
    pub fn new_from_host_with_filter<F>(filter: F) -> Self
    where
        F: FnMut(&Device<GlobalContext>) -> bool,
    {
        match rusb::devices() {
            Ok(list) => {
                let mut devs = vec![];
                for d in list.iter().filter(filter) {
                    devs.push(d)
                }
                Self {
                    available_devices: RwLock::new(Self::with_rusb_devices(devs)),
                    ..Default::default()
                }
            }
            Err(_) => Default::default(),
        }
    }
}
