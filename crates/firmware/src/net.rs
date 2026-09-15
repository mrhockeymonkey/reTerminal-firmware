//! WiFi station bring-up, DHCP, and the single `GET /screen` (design brief
//! §10). Plain HTTP over the LAN; no TLS.

use embassy_executor::Spawner;
use embassy_net::dns::DnsSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use embassy_net::{Config as NetConfig, Runner, Stack, StackResources};
use embassy_time::{Duration, with_timeout};
use esp_hal::peripherals::WIFI;
use esp_hal::rng::Rng;
use esp_radio::wifi::{Config as WifiConfig, Interface, StationConfig, WifiController};
use log::{info, warn};
use reqwless::client::HttpClient;
use reqwless::request::Method;
use static_cell::{ConstStaticCell, StaticCell};

use crate::config;

const WIFI_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DHCP_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetError {
    /// Radio could not be initialised or configured.
    Radio,
    /// No association within the timeout (wrong SSID/password, out of range).
    WifiConnect,
    /// Associated, but no DHCP lease within the timeout.
    Dhcp,
    /// TCP/HTTP failure or timeout talking to the server.
    Http,
    /// The server answered with a non-2xx status.
    Status(u16),
    /// The body did not fit the device's buffer.
    BodyTooLarge,
}

static STACK_RESOURCES: ConstStaticCell<StackResources<3>> =
    ConstStaticCell::new(StackResources::new());
static TCP_STATE: ConstStaticCell<TcpClientState<1, 4096, 4096>> =
    ConstStaticCell::new(TcpClientState::new());
static STACK: StaticCell<Stack<'static>> = StaticCell::new();

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface<'static>>) -> ! {
    runner.run().await
}

/// A connected station: the controller (kept so it can be shut down before
/// sleep) and the network stack.
pub struct Connection {
    pub controller: WifiController<'static>,
    pub stack: &'static Stack<'static>,
}

/// Joins the configured network and waits for a DHCP lease.
///
/// `esp_rtos::start` must already have been called.
pub async fn connect(spawner: &Spawner, wifi: WIFI<'static>) -> Result<Connection, NetError> {
    if config::is_placeholder_config() {
        warn!("WIFI_SSID/WIFI_PASSWORD were not set at build time; using placeholders");
    }

    // `wifi::new` starts the radio; `set_config` starts station mode.
    let (mut controller, interfaces) =
        esp_radio::wifi::new(wifi, Default::default()).map_err(|e| {
            warn!("wifi init: {e:?}");
            NetError::Radio
        })?;
    let station = StationConfig::default()
        .with_ssid(config::WIFI_SSID)
        .with_password(alloc::string::String::from(config::WIFI_PASSWORD));
    controller
        .set_config(&WifiConfig::Station(station))
        .map_err(|e| {
            warn!("wifi config: {e:?}");
            NetError::Radio
        })?;

    info!("wifi: connecting to {:?}", config::WIFI_SSID);
    match with_timeout(WIFI_CONNECT_TIMEOUT, controller.connect_async()).await {
        Ok(Ok(_)) => info!("wifi: connected"),
        Ok(Err(e)) => {
            warn!("wifi: connect failed: {e:?}");
            return Err(NetError::WifiConnect);
        }
        Err(_) => {
            warn!("wifi: connect timed out");
            return Err(NetError::WifiConnect);
        }
    }

    let rng = Rng::new();
    let seed = (u64::from(rng.random()) << 32) | u64::from(rng.random());
    let (stack, runner) = embassy_net::new(
        interfaces.station,
        NetConfig::dhcpv4(Default::default()),
        STACK_RESOURCES.take(),
        seed,
    );
    let stack = STACK.init(stack);
    spawner.spawn(net_task(runner)).expect("net task");

    match with_timeout(DHCP_TIMEOUT, stack.wait_config_up()).await {
        Ok(()) => {
            if let Some(v4) = stack.config_v4() {
                info!("dhcp: {}", v4.address);
            }
        }
        Err(_) => {
            warn!("dhcp: timed out");
            return Err(NetError::Dhcp);
        }
    }
    Ok(Connection { controller, stack })
}

/// Performs `GET SCREEN_URL`, leaving the body at the start of `buf` and
/// returning its length. `buf` also holds the response headers during the
/// request, so it must have some headroom beyond the largest expected body.
pub async fn fetch_screen(stack: &Stack<'static>, buf: &mut [u8]) -> Result<usize, NetError> {
    match with_timeout(HTTP_TIMEOUT, fetch_inner(stack, buf)).await {
        Ok(r) => r,
        Err(_) => {
            warn!("http: timed out");
            Err(NetError::Http)
        }
    }
}

async fn fetch_inner(stack: &Stack<'static>, buf: &mut [u8]) -> Result<usize, NetError> {
    let tcp = TcpClient::new(*stack, TCP_STATE.take());
    let dns = DnsSocket::new(*stack);
    let mut client = HttpClient::new(&tcp, &dns);

    info!("http: GET {}", config::SCREEN_URL);
    let mut request = client
        .request(Method::GET, config::SCREEN_URL)
        .await
        .map_err(|e| {
            warn!("http: connect/request failed: {e:?}");
            NetError::Http
        })?;
    let response = request.send(buf).await.map_err(|e| {
        warn!("http: send failed: {e:?}");
        NetError::Http
    })?;
    let status = response.status.0;
    if !response.status.is_successful() {
        warn!("http: status {status}");
        return Err(NetError::Status(status));
    }
    // `read_to_end` reads into the same buffer, after the headers, and
    // hands back the body slice; note where it sits so it can be moved to
    // the front once the borrow ends.
    let (start, len) = {
        let body = response.body().read_to_end().await.map_err(|e| {
            warn!("http: body read failed: {e:?}");
            match e {
                reqwless::Error::BufferTooSmall => NetError::BodyTooLarge,
                _ => NetError::Http,
            }
        })?;
        let start = body.as_ptr() as usize - buf.as_ptr() as usize;
        (start, body.len())
    };
    buf.copy_within(start..start + len, 0);
    info!("http: {len} byte body");
    Ok(len)
}

/// Disconnects and shuts the radio down (dropping the controller runs
/// `wifi_deinit`). Call before deep sleep.
pub async fn disconnect(conn: Connection) {
    let Connection { mut controller, .. } = conn;
    if let Err(e) = controller.disconnect_async().await {
        // `NotConnected` is expected when the association never happened.
        info!("wifi: disconnect: {e:?}");
    }
    drop(controller);
    info!("wifi: radio off");
}
