pub const NEED_GLOBAL_DICTIONARY_BIT: u64 = 1u64 << 8;
pub const HAS_ADDITIONAL_KEYS_BIT: u64 = 1u64 << 9;
pub const NEED_UPDATE_DICTIONARY_BIT: u64 = 1u64 << 10;

pub const TUINT8: u64 = 0;
pub const TUINT16: u64 = 1;
pub const TUINT32: u64 = 2;
pub const TUINT64: u64 = 3;

pub const LOW_CARDINALITY_VERSION: u64 = 1;

/// ClickHouse code: `DEFAULT_NATIVE_BINARY_MAX_NUM_COLUMNS`
pub const MAX_NUM_COLUMNS: usize = 1_000_000;

/// ClickHouse code: `DEFAULT_NATIVE_BINARY_MAX_NUM_ROWS`
pub const MAX_NUM_ROWS: usize = 1_000_000_000_000;

/// ClickHouse code: `ColumnDynamic::MAX_DYNAMIC_TYPES_LIMIT`
pub const MAX_DYNAMIC_TYPES: usize = 254;

/// Maximum parenthesis nesting accepted in a type header, including function parameters.
pub const MAX_TYPE_DEPTH: usize = 300;

/// Required stride alignment when a QBit dimension is split into multiple groups.
// Match `DataTypeQBit::DataTypeQBit` so malformed headers are rejected before decoding.
pub const QBIT_STRIDE_ALIGNMENT_BITS: u64 = 8;

/// Maximum number of stride groups accepted in a QBit dimension.
// Match ClickHouse's `DataTypeQBit::MAX_STRIDE_GROUPS`.
pub const QBIT_MAX_STRIDE_GROUPS: u64 = 1024;

/// ClickHouse code: `SerializationDynamic::SerializationVersion::V1`. Writes `max_dynamic_types`
/// before the type count; otherwise identical to V2.
pub const DYNAMIC_SERIALIZATION_V1: u64 = 1;
/// ClickHouse code: `SerializationDynamic::SerializationVersion::V2`
pub const DYNAMIC_SERIALIZATION_V2: u64 = 2;

/// ClickHouse code: `SerializationObject::SerializationVersion::V1`. Writes `max_dynamic_paths`
/// before the path count; otherwise identical to V2.
pub const JSON_SERIALIZATION_V1: u64 = 0;
/// ClickHouse code: `SerializationObject::SerializationVersion::V2`
pub const JSON_SERIALIZATION_V2: u64 = 2;
