//! 3.3.3.3 playerState shellshock fields from table 0xc55f70.

/// Shellshock state in the receiving client's CG time domain. The caller
/// supplies the current state for full and delta frames, and adds the replay
/// delay to a nonzero `begin_time`; `duration_msec` remains a duration.
/// Reset a named effect by retaining its index and zeroing begin and duration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShellShockSnapshot {
    /// PS+0x5ac, field105: four-bit index into the bound shock configuration
    /// table. Named entries use 1..=15; zero selects the native local override.
    pub index: u8,
    /// PS+0x5b0, field103: absolute milliseconds using codec -97. Zero resets
    /// a named effect and remains zero when the caller rebases replay time.
    pub begin_time: u32,
    /// PS+0x5b4, field104: sixteen-bit milliseconds in the hint-2 tail.
    pub duration_msec: u16,
}
