use bloch::FromBlock;
use bloch::reader::{
    Array, Bool, DateTime, F64, Ipv6, LcNullableTrustedStr, LcTrustedStr, Nullable, U32, U64, U128,
    Uuid,
};

/// `benchmark_sample`: a profiling event, see `testdata/benchmark_sample.sql`.
#[derive(FromBlock, Copy, Clone)]
pub struct BenchmarkCols<'a> {
    pub project: Uuid<'a>,
    pub kind: LcTrustedStr<'a>,
    pub timestamp: DateTime<'a>,
    pub count: F64<'a>,
    pub rate_hz: U32<'a>,

    pub host: LcNullableTrustedStr<'a>,
    pub thread: LcNullableTrustedStr<'a>,
    pub pod: LcNullableTrustedStr<'a>,
    pub container: LcNullableTrustedStr<'a>,
    pub namespace: LcNullableTrustedStr<'a>,
    pub version: LcNullableTrustedStr<'a>,
    pub kernel: LcNullableTrustedStr<'a>,

    pub host_ip: Nullable<'a, Ipv6<'a>>,

    pub service: LcNullableTrustedStr<'a>,
    pub tags: Array<'a, LcTrustedStr<'a>>,
    pub exe: LcNullableTrustedStr<'a>,

    #[col(name = "frames.kind")]
    pub frame_kinds: Array<'a, LcTrustedStr<'a>>,

    #[col(name = "frames.symbolized")]
    pub frame_symbolized: Array<'a, Bool<'a>>,

    #[col(name = "frames.file_id")]
    pub frame_file_ids: Array<'a, U128<'a>>,

    #[col(name = "frames.addr")]
    pub frame_addrs: Array<'a, U64<'a>>,
}
