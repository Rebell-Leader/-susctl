//! A self-contained tray icon with menus.

use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use ksni::{Handle, Icon, TrayMethods};
use log::{info, warn};
use rog_platform::platform::Properties;
use supergfxctl::pci_device::{Device, GfxMode, GfxPower};
use supergfxctl::zbus_proxy::DaemonProxy as GfxProxy;
use versions::Versioning;

use crate::config::Config;
use crate::zbus_proxies::{AppState, ROGCCZbusProxyBlocking};

const TRAY_LABEL: &str = "ROG Control Center";
const TRAY_ICON_PATH: &str = "/usr/share/icons/hicolor/512x512/apps/";

struct Icons {
    rog_blue: Icon,
    rog_red: Icon,
    rog_green: Icon,
    rog_white: Icon,
    gpu_integrated: Icon,
}

static ICONS: OnceLock<Icons> = OnceLock::new();

fn read_icon(file: &Path) -> Icon {
    let mut path = PathBuf::from(TRAY_ICON_PATH);
    path.push(file);
    let mut file = OpenOptions::new()
        .read(true)
        .open(&path)
        .unwrap_or_else(|_| panic!("Missing icon: {:?}", path));
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();

    let mut img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .expect("icon not found")
        .to_rgba8();
    for image::Rgba(pixel) in img.pixels_mut() {
        // (╯°□°）╯︵ ┻━┻
        *pixel = u32::from_be_bytes(*pixel).rotate_right(8).to_be_bytes();
    }

    let (width, height) = img.dimensions();
    Icon {
        width: width as i32,
        height: height as i32,
        data: img.into_raw(),
    }
}

struct AsusTray {
    current_title: String,
    current_icon: Icon,
    proxy: ROGCCZbusProxyBlocking<'static>,
    tray_channel: Option<tokio::sync::mpsc::UnboundedSender<TrayEvent>>,
    // System stats for native tooltip
    cpu_temp: String,
    gpu_temp: String,
    cpu_fan: String,
    gpu_fan: String,
    power_w: String,
    power_profile: String,
}

#[derive(Debug, Clone, Copy)]
pub enum TrayEvent {
    ToggleTooltip(i32, i32),
}

#[derive(Debug, Clone, Default)]
pub struct TrayStats {
    pub cpu_temp: String,
    pub gpu_temp: String,
    pub cpu_fan: String,
    pub gpu_fan: String,
    pub power_w: String,
    pub power_profile: String,
}

impl ksni::Tray for AsusTray {
    fn id(&self) -> String {
        TRAY_LABEL.into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![self.current_icon.clone()]
    }

    fn title(&self) -> String {
        self.current_title.clone()
    }

    fn status(&self) -> ksni::Status {
        ksni::Status::Active
    }

    fn activate(&mut self, x: i32, y: i32) {
        if let Some(tx) = &self.tray_channel {
            let _ = tx.send(TrayEvent::ToggleTooltip(x, y));
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "ROG Control Center".into(),
            description: format!(
                "<b>Profile:</b> {}\n<b>CPU:</b> {}°C | <b>GPU:</b> {}°C\n<b>Fans:</b> {} / {} RPM\n<b>Power:</b> {} W",
                self.power_profile,
                self.cpu_temp, self.gpu_temp,
                self.cpu_fan, self.gpu_fan,
                self.power_w
            ),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        vec![
            StandardItem {
                label: "Open ROGCC".into(),
                icon_name: "rog-control-center".into(),
                activate: Box::new(move |s: &mut AsusTray| {
                    s.proxy.set_state(AppState::MainWindowShouldOpen).ok();
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit ROGCC".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

async fn set_tray_icon_and_tip(
    mode: GfxMode,
    power: GfxPower,
    tray: &mut Handle<AsusTray>,
    supergfx_active: bool,
) {
    if let Some(icons) = ICONS.get() {
        let icon = match power {
            GfxPower::Suspended => icons.rog_blue.clone(),
            GfxPower::Off => {
                if mode == GfxMode::Vfio {
                    icons.rog_red.clone()
                } else {
                    icons.rog_green.clone()
                }
            }
            GfxPower::AsusDisabled => icons.rog_white.clone(),
            GfxPower::AsusMuxDiscreet | GfxPower::Active => icons.rog_red.clone(),
            GfxPower::Unknown => {
                if supergfx_active {
                    icons.gpu_integrated.clone()
                } else {
                    icons.rog_red.clone()
                }
            }
        };

        tray.update(|tray: &mut AsusTray| {
            tray.current_icon = icon;
            tray.current_title = format!(
                "ROG: gpu mode = {mode:?}, gpu power =
            {power:?}"
            );
        })
        .await;
    }
}

fn find_dgpu() -> Option<Device> {
    use supergfxctl::pci_device::Device;
    let dev = Device::find().unwrap_or_default();
    for dev in dev {
        if dev.is_dgpu() {
            info!("Found dGPU: {}", dev.pci_id());
            // Plain old thread is perfectly fine since most of this is potentially blocking
            return Some(dev);
        }
    }
    warn!("Did not find a dGPU on this system, dGPU status won't be avilable");
    None
}

/// The tray is controlled somewhat by `Arc<Mutex<SystemState>>`
pub fn init_tray(
    _supported_properties: Vec<Properties>,
    config: Arc<Mutex<Config>>,
    tray_channel: tokio::sync::mpsc::UnboundedSender<TrayEvent>,
    mut stats_rx: tokio::sync::watch::Receiver<TrayStats>,
) {
    tokio::spawn(async move {
        let user_con = zbus::blocking::Connection::session().unwrap();
        let proxy = ROGCCZbusProxyBlocking::new(&user_con).unwrap();

        let rog_red = read_icon(&PathBuf::from("asus_notif_red.png"));

        let tray_init = AsusTray {
            current_title: TRAY_LABEL.to_string(),
            current_icon: rog_red.clone(),
            proxy,
            tray_channel: Some(tray_channel),
            // Initialize stats fields
            cpu_temp: "--".into(),
            gpu_temp: "--".into(),
            cpu_fan: "--".into(),
            gpu_fan: "--".into(),
            power_w: "--".into(),
            power_profile: "Unknown".into(),
        };

        // TODO: return an error to the UI
        let mut tray;
        match tray_init.disable_dbus_name(true).spawn().await {
            Ok(t) => tray = t,
            Err(e) => {
                log::error!(
                    "Tray unable to be initialised: {e:?}. Do you have a system tray enabled?"
                );
                return;
            }
        }

        info!("Tray started");
        let rog_blue = read_icon(&PathBuf::from("asus_notif_blue.png"));
        let rog_green = read_icon(&PathBuf::from("asus_notif_green.png"));
        let rog_white = read_icon(&PathBuf::from("asus_notif_white.png"));
        let gpu_integrated = read_icon(&PathBuf::from("rog-control-center.png"));
        ICONS.get_or_init(|| Icons {
            rog_blue,
            rog_red: rog_red.clone(),
            rog_green,
            rog_white,
            gpu_integrated,
        });

        let mut has_supergfx = false;
        let conn = zbus::Connection::system().await.unwrap();
        if let Ok(gfx_proxy) = GfxProxy::new(&conn).await {
            match gfx_proxy.mode().await {
                Ok(_) => {
                    has_supergfx = true;
                    if let Ok(version) = gfx_proxy.version().await {
                        if let Some(version) = Versioning::new(&version) {
                            let curr_gfx = Versioning::new("5.2.0").unwrap();
                            warn!("supergfxd version = {version}");
                            if version < curr_gfx {
                                // Don't allow mode changing if too old a version
                                warn!("supergfxd found but is too old to use");
                                has_supergfx = false;
                            }
                        }
                    }
                }
                Err(e) => match e {
                    zbus::Error::MethodError(_, _, message) => {
                        warn!(
                            "Couldn't get mode from supergfxd: {message:?}, the supergfxd service \
                             may not be running or installed"
                        )
                    }
                    _ => warn!("Couldn't get mode from supergfxd: {e:?}"),
                },
            }

            info!("Started ROGTray");
            let mut last_power = GfxPower::Unknown;
            let dev = find_dgpu();

            // Loop with select! to handle both periodic checks and stats updates
            loop {
                tokio::select! {
                    _ = stats_rx.changed() => {
                        let stats = stats_rx.borrow().clone();
                        tray.update(move |t| {
                            t.cpu_temp = stats.cpu_temp;
                            t.gpu_temp = stats.gpu_temp;
                            t.cpu_fan = stats.cpu_fan;
                            t.gpu_fan = stats.gpu_fan;
                            t.power_w = stats.power_w;
                            t.power_profile = stats.power_profile;
                        }).await;
                    }
                    _ = tokio::time::sleep(Duration::from_millis(1000)) => {
                        if let Ok(lock) = config.try_lock() {
                            if !lock.enable_tray_icon {
                                return;
                            }
                        }
                        // Handle GPU icon updates
                        if has_supergfx {
                            if let Ok(mode) = gfx_proxy.mode().await {
                                if let Ok(power) = gfx_proxy.power().await {
                                    if last_power != power {
                                        set_tray_icon_and_tip(mode, power, &mut tray, has_supergfx).await;
                                        last_power = power;
                                    }
                                }
                            }
                        } else if let Some(dev) = dev.as_ref() {
                            if let Ok(power) = dev.get_runtime_status() {
                                if last_power != power {
                                    set_tray_icon_and_tip(GfxMode::Hybrid, power, &mut tray, has_supergfx)
                                        .await;
                                    last_power = power;
                                }
                            }
                        }
                    }
                }
            }
        }
    });
}
