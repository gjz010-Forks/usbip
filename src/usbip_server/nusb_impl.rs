use std::sync::{Arc, Mutex};

use log::*;

use crate::{
    EndpointAttributes, NusbUsbHostDeviceHandler, NusbUsbHostInterfaceHandler, UsbDevice,
    UsbEndpoint, UsbInterface, UsbInterfaceHandler, UsbIpServer,
};

impl UsbIpServer {
    /// Create a [UsbIpServer] with Vec<[nusb::DeviceInfo]> for sharing host devices
    pub fn with_nusb_devices(nusb_device_infos: Vec<nusb::DeviceInfo>) -> Vec<UsbDevice> {
        let mut devices = vec![];
        for device_info in nusb_device_infos {
            let dev = match device_info.open() {
                Ok(dev) => dev,
                Err(err) => {
                    warn!("Impossible to open device {device_info:?}: {err}, ignoring device",);
                    continue;
                }
            };
            let cfg = match dev.active_configuration() {
                Ok(cfg) => cfg,
                Err(err) => {
                    warn!(
                        "Impossible to get active configuration {device_info:?}: {err}, ignoring device",
                    );
                    continue;
                }
            };
            let mut interfaces = vec![];
            for intf in cfg.interfaces() {
                // ignore alternate settings
                let intf_num = intf.interface_number();
                let intf = dev.claim_interface(intf_num).unwrap();
                let alt_setting = intf.descriptors().next().unwrap();
                let mut endpoints = vec![];

                for ep_desc in alt_setting.endpoints() {
                    endpoints.push(UsbEndpoint {
                        address: ep_desc.address(),
                        attributes: ep_desc.transfer_type() as u8,
                        max_packet_size: ep_desc.max_packet_size() as u16,
                        interval: ep_desc.interval(),
                    });
                }

                let handler = Arc::new(Mutex::new(Box::new(NusbUsbHostInterfaceHandler::new(
                    Arc::new(Mutex::new(intf.clone())),
                ))
                    as Box<dyn UsbInterfaceHandler + Send>));
                interfaces.push(UsbInterface {
                    interface_class: alt_setting.class(),
                    interface_subclass: alt_setting.subclass(),
                    interface_protocol: alt_setting.protocol(),
                    endpoints,
                    string_interface: alt_setting.string_index().unwrap_or(0),
                    class_specific_descriptor: Vec::new(),
                    handler,
                });
            }
            let mut device = UsbDevice {
                path: format!(
                    "/sys/bus/{}/{}/{}",
                    device_info.bus_number(),
                    device_info.device_address(),
                    0
                ),
                bus_id: format!(
                    "{}-{}-{}",
                    device_info.bus_number(),
                    device_info.device_address(),
                    0,
                ),
                bus_num: device_info.bus_number() as u32,
                dev_num: 0,
                speed: device_info.speed().unwrap() as u32,
                vendor_id: device_info.vendor_id(),
                product_id: device_info.product_id(),
                device_class: device_info.class(),
                device_subclass: device_info.subclass(),
                device_protocol: device_info.protocol(),
                device_bcd: device_info.device_version().into(),
                configuration_value: cfg.configuration_value(),
                num_configurations: dev.configurations().count() as u8,
                ep0_in: UsbEndpoint {
                    address: 0x80,
                    attributes: EndpointAttributes::Control as u8,
                    max_packet_size: 16,
                    interval: 0,
                },
                ep0_out: UsbEndpoint {
                    address: 0x00,
                    attributes: EndpointAttributes::Control as u8,
                    max_packet_size: 16,
                    interval: 0,
                },
                interfaces,
                device_handler: Some(Arc::new(Mutex::new(Box::new(
                    NusbUsbHostDeviceHandler::new(Arc::new(Mutex::new(dev))),
                )))),
                ..UsbDevice::default()
            };

            // set strings
            if let Some(s) = device_info.manufacturer_string() {
                device.string_manufacturer = device.new_string(s)
            }
            if let Some(s) = device_info.product_string() {
                device.string_product = device.new_string(s)
            }
            if let Some(s) = device_info.serial_number() {
                device.string_serial = device.new_string(s)
            }
            devices.push(device);
        }
        devices
    }
}
