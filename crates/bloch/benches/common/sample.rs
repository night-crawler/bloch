use std::net::Ipv6Addr;

use chrono::Utc;

/// The `benchmark_sample` row as `clickhouse-rs` sees it over RowBinary.
#[derive(clickhouse::Row, serde::Serialize, serde::Deserialize, Debug)]
pub struct BenchmarkSample<'a> {
    #[serde(with = "clickhouse::serde::uuid")]
    pub project: uuid::Uuid,

    pub kind: &'a str,

    #[serde(with = "clickhouse::serde::chrono::datetime")]
    pub timestamp: chrono::DateTime<Utc>,

    pub count: f64,
    pub rate_hz: u32,

    pub host: Option<&'a str>,
    pub thread: Option<&'a str>,
    pub pod: Option<&'a str>,
    pub container: Option<&'a str>,
    pub namespace: Option<&'a str>,
    pub version: Option<&'a str>,
    pub kernel: Option<&'a str>,

    pub host_ip: Option<Ipv6Addr>,

    pub service: Option<&'a str>,
    pub tags: Vec<&'a str>,
    pub exe: Option<&'a str>,

    #[serde(rename = "frames.kind")]
    pub frame_kinds: Vec<&'a str>,

    #[serde(rename = "frames.symbolized")]
    pub frame_symbolized: Vec<bool>,

    #[serde(rename = "frames.file_id")]
    pub frame_file_ids: Vec<u128>,

    #[serde(rename = "frames.addr")]
    pub frame_addrs: Vec<u64>,
}
