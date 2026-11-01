//! Polar H10 (or any BLE heart-rate strap) via the standard Heart Rate service.
//!
//! Streams Heart Rate Measurement (0x2A37) notifications, which carry the
//! beat-to-beat RR intervals needed for HRV. The H10's accelerometer (Polar PMD
//! service) comes later.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use btleplug::api::{
    Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter, bleuuid::uuid_from_u16,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use model::{Clock, Device, Event, EventKind, Heart, Link};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub const HR_SERVICE: Uuid = uuid_from_u16(0x180D);
pub const HR_MEASUREMENT: Uuid = uuid_from_u16(0x2A37);

/// Parse a Heart Rate Measurement value.
///
/// Flags (byte 0): bit 0 = 16-bit HR, bits 1-2 = sensor contact
/// (bit 2 supported, bit 1 detected), bit 3 = energy expended present,
/// bit 4 = RR intervals present (u16 LE each, 1/1024 s).
pub fn parse_hr_measurement(d: &[u8]) -> Option<Heart> {
    let flags = *d.first()?;
    let (bpm, mut i) = if flags & 0x01 != 0 {
        (u16::from_le_bytes([*d.get(1)?, *d.get(2)?]), 3)
    } else {
        (*d.get(1)? as u16, 2)
    };
    let contact = match (flags >> 1) & 0b11 {
        0b10 => Some(false),
        0b11 => Some(true),
        _ => None,
    };
    if flags & 0x08 != 0 {
        i += 2;
    }
    let mut rr_1024 = Vec::new();
    if flags & 0x10 != 0 {
        while i + 1 < d.len() {
            rr_1024.push(u16::from_le_bytes([d[i], d[i + 1]]));
            i += 2;
        }
    }
    Some(Heart { bpm, rr_1024, contact })
}

#[derive(Debug, Clone)]
pub struct H10Config {
    /// Connect to the first HR device whose name contains this (case-insensitive).
    /// Empty matches any heart-rate device.
    pub name_contains: String,
    /// Reconnect if no notification arrives for this long.
    pub silence_timeout: Duration,
}

impl Default for H10Config {
    fn default() -> Self {
        Self {
            name_contains: "H10".into(),
            silence_timeout: Duration::from_secs(10),
        }
    }
}

enum End {
    /// Receiver dropped: stop for good.
    Stop,
    /// Strap went away: scan again.
    Lost,
}

/// Stream heart rate until `tx` is closed, reconnecting when the strap drops.
/// Disconnects cleanly on the way out so the strap is free for other devices.
pub async fn run(clock: Clock, tx: mpsc::Sender<Event>, cfg: H10Config) {
    let adapter = match first_adapter().await {
        Ok(a) => a,
        Err(e) => {
            info!("H10 disabled: {e:#}");
            send_link(&tx, &clock, false, Some(format!("{e:#}"))).await;
            return;
        }
    };
    // Retries every few seconds; only log when the reason changes.
    let mut last_err: Option<String> = None;
    loop {
        match session(&adapter, &clock, &tx, &cfg).await {
            Ok(End::Stop) => return,
            Ok(End::Lost) => {
                last_err = None;
                if !send_link(&tx, &clock, false, Some("signal lost".into())).await {
                    return;
                }
            }
            Err(e) => {
                let msg = format!("{e:#}");
                if last_err.as_ref() != Some(&msg) {
                    warn!("H10: {msg}");
                }
                last_err = Some(msg.clone());
                if !send_link(&tx, &clock, false, Some(msg)).await {
                    return;
                }
            }
        }
        tokio::select! {
            _ = tx.closed() => return,
            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
        }
    }
}

async fn first_adapter() -> Result<Adapter> {
    let manager = Manager::new().await.context("Bluetooth unavailable")?;
    manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no Bluetooth adapter"))
}

async fn session(adapter: &Adapter, clock: &Clock, tx: &mpsc::Sender<Event>, cfg: &H10Config) -> Result<End> {
    let Some((p, name)) = find(adapter, tx, cfg).await? else {
        return Ok(End::Stop);
    };
    adapter.stop_scan().await.ok();
    p.connect().await.with_context(|| format!("connect {name}"))?;
    p.discover_services()
        .await
        .with_context(|| format!("discover services on {name}"))?;
    let ch = p
        .characteristics()
        .into_iter()
        .find(|c| c.uuid == HR_MEASUREMENT)
        .ok_or_else(|| anyhow!("{name} has no HR measurement characteristic"))?;
    p.subscribe(&ch)
        .await
        .with_context(|| format!("subscribe to heart rate on {name}"))?;
    info!("H10 connected: {name}");
    if !send_link(tx, clock, true, Some(name.clone())).await {
        disconnect(&p, &ch).await;
        return Ok(End::Stop);
    }

    let mut notes = p.notifications().await?;
    let mut central = adapter.events().await?;
    loop {
        tokio::select! {
            _ = tx.closed() => {
                disconnect(&p, &ch).await;
                return Ok(End::Stop);
            }
            note = tokio::time::timeout(cfg.silence_timeout, notes.next()) => match note {
                Err(_) => {
                    warn!("H10: no data for {:?}, reconnecting", cfg.silence_timeout);
                    disconnect(&p, &ch).await;
                    return Ok(End::Lost);
                }
                Ok(None) => return Ok(End::Lost),
                Ok(Some(n)) if n.uuid == HR_MEASUREMENT => {
                    let t_us = clock.now_us();
                    if let Some(heart) = parse_hr_measurement(&n.value) {
                        let ev = Event { t_us, kind: EventKind::Heart(heart) };
                        if tx.send(ev).await.is_err() {
                            disconnect(&p, &ch).await;
                            return Ok(End::Stop);
                        }
                    }
                }
                Ok(Some(_)) => {}
            },
            ev = central.next() => {
                if let Some(CentralEvent::DeviceDisconnected(id)) = ev && id == p.id() {
                    return Ok(End::Lost);
                }
            }
        }
    }
}

/// Scan until a matching strap shows up. `None` if `tx` closed first.
async fn find(adapter: &Adapter, tx: &mpsc::Sender<Event>, cfg: &H10Config) -> Result<Option<(Peripheral, String)>> {
    // Subscribe before scanning so no advertisement is missed.
    let mut events = adapter.events().await?;
    adapter
        .start_scan(ScanFilter {
            services: vec![HR_SERVICE],
        })
        .await
        .context("start BLE scan (Bluetooth off or blocked? try `rfkill unblock bluetooth && bluetoothctl power on`)")?;
    debug!("H10: scanning");
    // BlueZ may already know the strap from an earlier session.
    for p in adapter.peripherals().await? {
        if let Some(name) = matching_name(&p, cfg).await {
            return Ok(Some((p, name)));
        }
    }
    loop {
        tokio::select! {
            _ = tx.closed() => {
                adapter.stop_scan().await.ok();
                return Ok(None);
            }
            ev = events.next() => match ev {
                Some(CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id)) => {
                    let p = adapter.peripheral(&id).await?;
                    if let Some(name) = matching_name(&p, cfg).await {
                        return Ok(Some((p, name)));
                    }
                }
                Some(_) => {}
                None => return Err(anyhow!("Bluetooth event stream ended")),
            }
        }
    }
}

async fn matching_name(p: &Peripheral, cfg: &H10Config) -> Option<String> {
    let props = p.properties().await.ok()??;
    if !props.services.contains(&HR_SERVICE) {
        return None;
    }
    let name = props.local_name?;
    name.to_lowercase()
        .contains(&cfg.name_contains.to_lowercase())
        .then_some(name)
}

async fn disconnect(p: &Peripheral, ch: &Characteristic) {
    p.unsubscribe(ch).await.ok();
    p.disconnect().await.ok();
}

async fn send_link(tx: &mpsc::Sender<Event>, clock: &Clock, connected: bool, detail: Option<String>) -> bool {
    let kind = EventKind::Link(Link {
        device: Device::H10,
        connected,
        detail,
    });
    tx.send(Event { t_us: clock.now_us(), kind }).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_8bit_hr_with_rr() {
        // Typical H10 packet: contact supported + detected, RR present.
        let h = parse_hr_measurement(&[0x16, 72, 0x2A, 0x03, 0x30, 0x03]).unwrap();
        assert_eq!(h.bpm, 72);
        assert_eq!(h.contact, Some(true));
        assert_eq!(h.rr_1024, vec![0x032A, 0x0330]);
    }

    #[test]
    fn parses_16bit_hr_and_skips_energy_expended() {
        // 16-bit HR (0x00B4 = 180), energy expended (2 bytes), one RR.
        let h = parse_hr_measurement(&[0x19, 0xB4, 0x00, 0xFF, 0xFF, 0x00, 0x02]).unwrap();
        assert_eq!(h.bpm, 180);
        assert_eq!(h.rr_1024, vec![512]);
        assert_eq!(h.contact, None);
    }

    #[test]
    fn reports_lost_contact() {
        let h = parse_hr_measurement(&[0x04, 0]).unwrap();
        assert_eq!(h.contact, Some(false));
        assert!(h.rr_1024.is_empty());
    }

    #[test]
    fn rejects_short_packets() {
        assert!(parse_hr_measurement(&[]).is_none());
        assert!(parse_hr_measurement(&[0x01, 0x48]).is_none());
    }
}
