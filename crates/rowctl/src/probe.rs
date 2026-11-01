//! `pm5-probe`: confirm command codes and byte order on a real PM5.

use anyhow::Result;
use devices::csafe::{Item, pm_cmd};
use devices::pm5::{self, Exchange};

pub async fn run() -> Result<()> {
    let results = tokio::task::spawn_blocking(pm5::probe).await??;
    for (label, ex) in &results {
        print_exchange(label, ex);
    }
    println!(
        "\nRun this twice: once with the PM5 awake on its main menu, and once while\n\
         rowing gently (force data only fills during a drive). Multi-byte values show\n\
         both byte orders; the driver assumes little-endian (le)."
    );
    Ok(())
}

/// The PM's workout type codes (published table; to be confirmed).
fn workout_type(code: u8) -> &'static str {
    match code {
        0 => "just row, no splits",
        1 => "just row, splits",
        2 => "fixed distance, no splits",
        3 => "fixed distance, splits",
        4 => "fixed time, no splits",
        5 => "fixed time, splits",
        6 => "fixed time intervals",
        7 => "fixed distance intervals",
        8 => "variable intervals",
        _ => "other",
    }
}

pub async fn workout(minutes: u32) -> Result<()> {
    println!(
        "This sets a {minutes}:00 workout on the PM5 (press Menu on the PM5 to cancel it).\n\
         Run it with the PM5 awake on its main menu.\n"
    );
    let results = tokio::task::spawn_blocking(move || pm5::workout_probe(minutes * 60)).await??;
    for (label, ex) in &results {
        print_exchange(label, ex);
        if let Some(r) = &ex.response
            && let Some(t) = r.items.iter().find(|i| i.pm && i.code == pm_cmd::GET_WORKOUT_TYPE)
            && let Some(&code) = t.data.first()
        {
            println!("  → workout type {code}: {}", workout_type(code));
        }
    }
    println!(
        "\nIt worked if a step's status shows previous frame Ok and the type afterwards is\n\
         fixed time (4 or 5), with the PM5 showing the workout. Paste this output either way."
    );
    Ok(())
}

fn print_exchange(label: &str, ex: &Exchange) {
    println!("{label}");
    println!("  tx {}", hex(&ex.tx));
    println!("  rx {}", if ex.rx.is_empty() { "(nothing)".into() } else { hex(&ex.rx) });
    let Some(r) = &ex.response else {
        println!("  no matching reply");
        return;
    };
    println!(
        "  status {:#04x}: previous frame {:?}, state {}",
        r.status,
        r.prev_frame(),
        r.machine_state()
    );
    if r.items.is_empty() {
        println!("  (no data items)");
    }
    for item in &r.items {
        println!("  {}", describe(item));
    }
}

fn describe(item: &Item) -> String {
    let space = if item.pm { "pm " } else { "std" };
    let head = format!("{space} {:#04x} [{:>2}] {}", item.code, item.data.len(), hex(&item.data));
    if item.pm && item.code == pm_cmd::GET_FORCE_PLOT_DATA {
        let Some((&n, data)) = item.data.split_first() else {
            return head;
        };
        let (pairs, _) = data[..(n as usize).min(data.len())].as_chunks::<2>();
        let pts = |be: bool| -> Vec<u16> {
            pairs
                .iter()
                .map(|&p| if be { u16::from_be_bytes(p) } else { u16::from_le_bytes(p) })
                .collect()
        };
        return format!("{head}\n      bytes read {n}; points le {:?}\n                       be {:?}", pts(false), pts(true));
    }
    let d = &item.data;
    let mut parts = vec![head];
    if d.len() >= 2 {
        parts.push(format!(
            "u16 le {} / be {}",
            u16::from_le_bytes([d[0], d[1]]),
            u16::from_be_bytes([d[0], d[1]])
        ));
    }
    if d.len() >= 4 {
        parts.push(format!(
            "u32 le {} / be {}",
            u32::from_le_bytes([d[0], d[1], d[2], d[3]]),
            u32::from_be_bytes([d[0], d[1], d[2], d[3]])
        ));
    }
    parts.join("   ")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}
