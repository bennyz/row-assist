//! Concept2 logbook CSV export ("Season" download from log.concept2.com).

use std::io::Read;

use anyhow::{Context, Result, anyhow};
use chrono::NaiveDateTime;
use serde::Deserialize;

pub const DATE_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// One logbook result.
#[derive(Debug, Clone, PartialEq)]
pub struct C2Result {
    pub log_id: i64,
    pub started_at: NaiveDateTime,
    pub description: String,
    pub work_time_s: f64,
    pub rest_time_s: Option<f64>,
    pub work_distance_m: i64,
    pub rest_distance_m: Option<i64>,
    pub stroke_rate: Option<f64>,
    /// Includes strokes taken during rest intervals.
    pub stroke_count: Option<i64>,
    pub pace_s_per_500: Option<f64>,
    pub avg_watts: Option<f64>,
    pub cal_per_hour: Option<f64>,
    pub total_cal: Option<f64>,
    pub avg_hr: Option<i64>,
    pub drag_factor: Option<i64>,
    pub age: Option<i64>,
    pub weight_class: Option<String>,
    pub machine: String,
    pub ranked: bool,
    pub comments: Option<String>,
    pub entered_at: Option<NaiveDateTime>,
}

#[derive(Debug, Deserialize)]
struct Row {
    #[serde(rename = "Log ID")]
    log_id: i64,
    #[serde(rename = "Date")]
    date: String,
    #[serde(rename = "Description")]
    description: String,
    #[serde(rename = "Work Time (Seconds)")]
    work_time_s: f64,
    #[serde(rename = "Rest Time (Seconds)")]
    rest_time_s: Option<f64>,
    #[serde(rename = "Work Distance")]
    work_distance_m: i64,
    #[serde(rename = "Rest Distance")]
    rest_distance_m: Option<i64>,
    #[serde(rename = "Stroke Rate/Cadence")]
    stroke_rate: Option<f64>,
    #[serde(rename = "Stroke Count")]
    stroke_count: Option<i64>,
    #[serde(rename = "Pace")]
    pace: String,
    #[serde(rename = "Avg Watts")]
    avg_watts: Option<f64>,
    #[serde(rename = "Cal/Hour")]
    cal_per_hour: Option<f64>,
    #[serde(rename = "Total Cal")]
    total_cal: Option<f64>,
    #[serde(rename = "Avg Heart Rate")]
    avg_hr: Option<i64>,
    #[serde(rename = "Drag Factor")]
    drag_factor: Option<i64>,
    #[serde(rename = "Age")]
    age: Option<i64>,
    #[serde(rename = "Weight")]
    weight: String,
    #[serde(rename = "Type")]
    machine: String,
    #[serde(rename = "Ranked")]
    ranked: String,
    #[serde(rename = "Comments")]
    comments: String,
    #[serde(rename = "Date Entered")]
    date_entered: String,
}

pub fn parse_csv(r: impl Read) -> Result<Vec<C2Result>> {
    let mut reader = csv::Reader::from_reader(r);
    reader
        .deserialize::<Row>()
        .enumerate()
        .map(|(i, row)| {
            let row = row.with_context(|| format!("CSV row {}", i + 2))?;
            convert(row).with_context(|| format!("CSV row {}", i + 2))
        })
        .collect()
}

fn convert(r: Row) -> Result<C2Result> {
    Ok(C2Result {
        log_id: r.log_id,
        started_at: NaiveDateTime::parse_from_str(&r.date, DATE_FORMAT)
            .with_context(|| format!("bad date {:?}", r.date))?,
        description: r.description,
        work_time_s: r.work_time_s,
        rest_time_s: r.rest_time_s,
        work_distance_m: r.work_distance_m,
        rest_distance_m: r.rest_distance_m,
        stroke_rate: r.stroke_rate,
        stroke_count: r.stroke_count,
        pace_s_per_500: parse_pace(&r.pace)?,
        avg_watts: r.avg_watts,
        cal_per_hour: r.cal_per_hour,
        total_cal: r.total_cal,
        // The export writes 0 when no heart-rate monitor was connected.
        avg_hr: r.avg_hr.filter(|&hr| hr > 0),
        drag_factor: r.drag_factor.filter(|&df| df > 0),
        age: r.age,
        weight_class: non_empty(r.weight),
        machine: r.machine,
        ranked: r.ranked.eq_ignore_ascii_case("yes"),
        comments: non_empty(r.comments),
        entered_at: NaiveDateTime::parse_from_str(&r.date_entered, DATE_FORMAT).ok(),
    })
}

/// `m:ss.t` → seconds. Empty → `None`.
pub fn parse_pace(s: &str) -> Result<Option<f64>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let (min, sec) = s.split_once(':').ok_or_else(|| anyhow!("bad pace {s:?}"))?;
    let min: f64 = min.parse().with_context(|| format!("bad pace {s:?}"))?;
    let sec: f64 = sec.parse().with_context(|| format!("bad pace {s:?}"))?;
    Ok(Some(min * 60.0 + sec))
}

fn non_empty(s: String) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic rows in the export's exact format.
    const SAMPLE: &str = r#""Log ID",Date,Description,"Work Time (Formatted)","Work Time (Seconds)","Rest Time (Formatted)","Rest Time (Seconds)","Work Distance","Rest Distance","Stroke Rate/Cadence","Stroke Count",Pace,"Avg Watts",Cal/Hour,"Total Cal","Avg Heart Rate","Drag Factor",Age,Weight,Type,Ranked,Comments,"Date Entered"
1001,"2026-01-02 07:30:00","20:00 row",20:00.0,1200,,,4000,,20,400,2:30.0,104,700,230,140,110,30,Hwt,RowErg,No,,"2026-01-02 05:31:00"
1002,"2026-01-03 07:30:00","v1:00/1:00r...8 row",8:00.0,480,8:00.0,480,2000,900,22,330,2:00.0,203,1000,130,0,0,30,Hwt,RowErg,Yes,"Felt good, legs heavy","2026-01-03 05:31:00"
"#;

    #[test]
    fn parses_export_rows() {
        let rows = parse_csv(SAMPLE.as_bytes()).unwrap();
        assert_eq!(rows.len(), 2);

        let a = &rows[0];
        assert_eq!(a.log_id, 1001);
        assert_eq!(a.started_at.to_string(), "2026-01-02 07:30:00");
        assert_eq!(a.work_time_s, 1200.0);
        assert_eq!(a.rest_time_s, None);
        assert_eq!(a.rest_distance_m, None);
        assert_eq!(a.pace_s_per_500, Some(150.0));
        assert_eq!(a.avg_hr, Some(140));
        assert_eq!(a.drag_factor, Some(110));
        assert!(!a.ranked);
        assert_eq!(a.comments, None);

        let b = &rows[1];
        assert_eq!(b.rest_time_s, Some(480.0));
        assert_eq!(b.rest_distance_m, Some(900));
        assert_eq!(b.avg_hr, None, "0 means no monitor");
        assert_eq!(b.drag_factor, None);
        assert!(b.ranked);
        assert_eq!(b.comments.as_deref(), Some("Felt good, legs heavy"));
    }

    #[test]
    fn parses_paces() {
        assert_eq!(parse_pace("2:41.8").unwrap(), Some(161.8));
        assert_eq!(parse_pace("").unwrap(), None);
        assert!(parse_pace("abc").is_err());
    }

    #[test]
    fn reports_bad_rows_with_line_numbers() {
        let bad = SAMPLE.replace("2026-01-03 07:30:00", "yesterday");
        let err = format!("{:#}", parse_csv(bad.as_bytes()).unwrap_err());
        assert!(err.contains("CSV row 3"), "{err}");
    }
}
