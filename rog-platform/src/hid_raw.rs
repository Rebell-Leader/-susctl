use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

use libc;
use log::{info, warn};
use udev::Device;

use crate::error::{PlatformError, Result};

/// A USB device that utilizes hidraw for I/O
#[derive(Debug)]
pub struct HidRaw {
    /// The path to the `/dev/<name>` of the device
    devfs_path: PathBuf,
    /// The sysfs path
    syspath: PathBuf,
    /// The product ID. The vendor ID is not kept
    prod_id: String,
    _device_bcd: u32,
    /// Retaining a handle to the file for the duration of `HidRaw`
    file: RefCell<File>,
}

impl HidRaw {
    pub fn new(id_product: &str) -> Result<Self> {
        let mut enumerator = udev::Enumerator::new().map_err(|err| {
            warn!("{}", err);
            PlatformError::Udev("enumerator failed".into(), err)
        })?;

        enumerator.match_subsystem("hidraw").map_err(|err| {
            warn!("{}", err);
            PlatformError::Udev("match_subsystem failed".into(), err)
        })?;

        for endpoint in enumerator
            .scan_devices()
            .map_err(|e| PlatformError::IoPath("enumerator".to_owned(), e))?
        {
            if let Some(usb_device) = endpoint
                .parent_with_subsystem_devtype("usb", "usb_device")
                .map_err(|e| {
                    PlatformError::IoPath(endpoint.devpath().to_string_lossy().to_string(), e)
                })?
            {
                if let Some(dev_node) = endpoint.devnode() {
                    if let Some(this_id_product) = usb_device.attribute_value("idProduct") {
                        if this_id_product != id_product {
                            continue;
                        }
                        let dev_path = endpoint.devpath().to_string_lossy();
                        if dev_path.contains("virtual") {
                            info!(
                                "Using device at: {:?} for <TODO: label control> control",
                                dev_node
                            );
                        }
                        return Ok(Self {
                            file: RefCell::new(
                                OpenOptions::new().read(true).write(true).open(dev_node)?,
                            ),
                            devfs_path: dev_node.to_owned(),
                            prod_id: this_id_product.to_string_lossy().into(),
                            syspath: endpoint.syspath().into(),
                            _device_bcd: usb_device
                                .attribute_value("bcdDevice")
                                .unwrap_or_default()
                                .to_string_lossy()
                                .parse()
                                .unwrap_or_default(),
                        });
                    }
                }
            }
        }
        Err(PlatformError::MissingFunction(format!(
            "hidraw dev {} not found",
            id_product
        )))
    }

    /// Make `HidRaw` device from a udev device
    pub fn from_device(endpoint: Device) -> Result<Self> {
        if let Some(parent) = endpoint
            .parent_with_subsystem_devtype("usb", "usb_device")
            .map_err(|e| {
                PlatformError::IoPath(endpoint.devpath().to_string_lossy().to_string(), e)
            })?
        {
            if let Some(dev_node) = endpoint.devnode() {
                if let Some(id_product) = parent.attribute_value("idProduct") {
                    return Ok(Self {
                        file: RefCell::new(
                            OpenOptions::new().read(true).write(true).open(dev_node)?,
                        ),
                        devfs_path: dev_node.to_owned(),
                        prod_id: id_product.to_string_lossy().into(),
                        syspath: endpoint.syspath().into(),
                        _device_bcd: endpoint
                            .attribute_value("bcdDevice")
                            .unwrap_or_default()
                            .to_string_lossy()
                            .parse()
                            .unwrap_or_default(),
                    });
                }
            }
        }
        Err(PlatformError::MissingFunction(
            "hidraw dev no dev path".to_string(),
        ))
    }

    pub fn prod_id(&self) -> &str {
        &self.prod_id
    }

    /// Write an array of raw bytes to the device using the hidraw interface
    pub fn write_bytes(&self, message: &[u8]) -> Result<()> {
        if let Ok(mut file) = self.file.try_borrow_mut() {
            // TODO: re-get the file if error?
            file.write_all(message).map_err(|e| {
                PlatformError::IoPath(self.devfs_path.to_string_lossy().to_string(), e)
            })?;
        }
        Ok(())
    }

    /// Write a HID feature report via HIDIOCSFEATURE ioctl.
    ///
    /// This is required for feature reports; `write_bytes` only sends output reports.
    pub fn write_feature_report(&self, message: &[u8]) -> Result<()> {
        // HIDIOCSFEATURE(len) = _IOC(_IOC_WRITE|_IOC_READ, 'H', 6, len)
        // On Linux x86_64: dir[31:30]=3, size[29:16]=len, type[15:8]='H', nr[7:0]=6
        let len = message.len() as u64;
        let ioctl_num: libc::c_ulong =
            ((3u64 << 30) | ((b'H' as u64) << 8) | 6 | (len << 16)) as libc::c_ulong;

        let mut buf = message.to_vec();

        let fd = self
            .file
            .try_borrow()
            .map_err(|_| {
                PlatformError::MissingFunction("hidraw file borrow failed".into())
            })?
            .as_raw_fd();

        let ret = unsafe { libc::ioctl(fd, ioctl_num, buf.as_mut_ptr()) };
        if ret < 0 {
            return Err(PlatformError::IoPath(
                self.devfs_path.to_string_lossy().to_string(),
                std::io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    /// This method was added for certain devices like AniMe to prevent them
    /// waking the laptop
    pub fn set_wakeup_disabled(&self) -> Result<()> {
        let mut dev = Device::from_syspath(&self.syspath)?;
        Ok(dev.set_attribute_value("power/wakeup", "disabled")?)
    }
}
