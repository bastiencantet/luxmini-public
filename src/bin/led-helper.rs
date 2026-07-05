// Setuid-root helper for SMC writes.
// Protocol (line-based stdin/stdout):
//   PING                                      -> PONG
//   WRITE <KEY> <HEXBYTE> <HEXBYTE>...        -> OK / ERR <msg>
//   READ  <KEY>                               -> OK <HEXBYTE>... / ERR <msg>
//   LIST                                      -> OK count <N> \n <KEY>\n... / ERR <msg>
// Hex bytes are space-separated lowercase hex (max 32 bytes; the app sends exactly 2).
// The key arrives over IPC from the device profile and is NEVER hardcoded here.
// WRITE/READ surface the SMC `result` byte (e.g. 0x84 = key not found), so an "OK"
// genuinely means the SMC accepted the operation — not merely that the kernel call returned.

#[link(name = "IOKit", kind = "framework")]
extern "C" {}

use std::io::{self, BufRead, Write};

mod smc {
    use std::io;
    use std::mem;

    type IOReturn = i32;
    type MachPort = u32;
    type IOConnect = u32;
    type IOService = u32;
    const KERN_SUCCESS: IOReturn = 0;
    const KERNEL_INDEX_SMC: u32 = 2;
    // SMC selectors carried in SMCKeyData.data8.
    const SMC_CMD_READ_BYTES: u8 = 5;
    const SMC_CMD_WRITE_KEY: u8 = 6;
    const SMC_CMD_READ_INDEX: u8 = 8;
    const SMC_CMD_READ_KEY_INFO: u8 = 9;

    extern "C" {
        fn mach_task_self() -> MachPort;
        fn IOServiceMatching(name: *const u8) -> *mut std::ffi::c_void;
        fn IOServiceGetMatchingService(
            master_port: MachPort,
            matching: *mut std::ffi::c_void,
        ) -> IOService;
        fn IOServiceOpen(
            service: IOService,
            owning_task: MachPort,
            connect_type: u32,
            connection: *mut IOConnect,
        ) -> IOReturn;
        fn IOServiceClose(connection: IOConnect) -> IOReturn;
        fn IOConnectCallStructMethod(
            connection: IOConnect,
            selector: u32,
            input: *const u8,
            input_cnt: usize,
            output: *mut u8,
            output_cnt: *mut usize,
        ) -> IOReturn;
        fn IOObjectRelease(object: u32) -> IOReturn;
    }

    #[repr(C)]
    #[allow(clippy::struct_field_names)] // field names mirror Apple's SMCKeyInfoData_t C ABI
    struct SMCKeyInfoData {
        data_size: u32,
        data_type: u32,
        data_attributes: u8,
    }

    #[repr(C)]
    struct SMCKeyData {
        key: u32,
        vers: [u8; 6],
        p_limit_data: [u8; 16],
        key_info: SMCKeyInfoData,
        result: u8,
        status: u8,
        data8: u8,
        data32: u32,
        bytes: [u8; 32],
    }

    impl SMCKeyData {
        const fn new() -> Self {
            // SAFETY: SMCKeyData is repr(C) POD; all-zero is a valid value.
            unsafe { mem::zeroed() }
        }
    }

    /// Pack the first ≤4 bytes big-endian into a u32, zero-padding a short slice. No
    /// indexing, so it cannot panic on an over-long or empty input.
    pub fn be_u32_prefix(bytes: &[u8]) -> u32 {
        let mut k = [0u8; 4];
        for (dst, &b) in k.iter_mut().zip(bytes) {
            *dst = b;
        }
        u32::from_be_bytes(k)
    }

    /// Pack a (≤4 char) SMC key name into its big-endian u32 representation.
    pub fn key_to_u32(key: &str) -> u32 {
        be_u32_prefix(key.as_bytes())
    }

    /// Decode a big-endian u32 key field back into its 4-character name (trailing NULs
    /// trimmed). Inverse of [`key_to_u32`] for ASCII keys.
    pub fn key_name(raw: u32) -> String {
        raw.to_be_bytes()
            .iter()
            .filter(|&&b| b != 0)
            .map(|&b| b as char)
            .collect()
    }

    pub struct SmcConn(IOConnect);

    impl SmcConn {
        pub fn open() -> io::Result<Self> {
            // SAFETY: standard IOKit C ABI; pointers null/zero-checked, service released once.
            unsafe {
                let matching = IOServiceMatching(c"AppleSMC".as_ptr().cast());
                if matching.is_null() {
                    return Err(io::Error::new(io::ErrorKind::NotFound, "no AppleSMC"));
                }
                let service = IOServiceGetMatchingService(0, matching);
                if service == 0 {
                    return Err(io::Error::new(io::ErrorKind::NotFound, "no service"));
                }
                let mut conn: IOConnect = 0;
                let r = IOServiceOpen(service, mach_task_self(), 0, &raw mut conn);
                IOObjectRelease(service);
                if r != KERN_SUCCESS {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!("open failed: {r}"),
                    ));
                }
                Ok(Self(conn))
            }
        }

        fn call(&self, input: &SMCKeyData) -> io::Result<SMCKeyData> {
            // SAFETY: repr(C) SMCKeyData in/out; buffer sizes = sizeof, so the kernel can't overflow.
            unsafe {
                let mut output = SMCKeyData::new();
                let mut out_size = mem::size_of::<SMCKeyData>();
                let r = IOConnectCallStructMethod(
                    self.0,
                    KERNEL_INDEX_SMC,
                    std::ptr::from_ref(input).cast::<u8>(),
                    mem::size_of::<SMCKeyData>(),
                    (&raw mut output).cast::<u8>(),
                    &raw mut out_size,
                );
                if r != KERN_SUCCESS {
                    return Err(io::Error::other(format!("call failed: {r}")));
                }
                Ok(output)
            }
        }

        /// Read the key's info block (data size + type). Fails if the SMC reports a
        /// non-zero result (e.g. 0x84 = key not found), so callers can tell whether the
        /// key actually exists on this machine. NEVER hardcode a key — it arrives over IPC.
        fn key_info(&self, key_u32: u32) -> io::Result<SMCKeyInfoData> {
            let mut input = SMCKeyData::new();
            input.key = key_u32;
            input.data8 = SMC_CMD_READ_KEY_INFO;
            let info = self.call(&input)?;
            if info.result != 0 {
                return Err(io::Error::other(format!(
                    "smc key-info result 0x{:02x}",
                    info.result
                )));
            }
            Ok(info.key_info)
        }

        pub fn write_key(&self, key: &str, data: &[u8]) -> io::Result<()> {
            let key_u32 = key_to_u32(key);

            // First call: get key info (data_size). Surfaces "key not found" instead of
            // silently writing into a zero-sized key.
            let info = self.key_info(key_u32)?;

            // Second call: write.
            let mut input = SMCKeyData::new();
            input.key = key_u32;
            input.data8 = SMC_CMD_WRITE_KEY;
            input.key_info.data_size = info.data_size;
            // Copy up to 32 bytes (the fixed SMC buffer); zip bounds both sides, no panic.
            for (dst, &src) in input.bytes.iter_mut().zip(data) {
                *dst = src;
            }
            let out = self.call(&input)?;
            // SMC can reject the write even when the kernel call succeeds; OK requires result == 0.
            if out.result != 0 {
                return Err(io::Error::other(format!(
                    "smc write result 0x{:02x}",
                    out.result
                )));
            }
            Ok(())
        }

        /// Number of SMC keys on this machine (the value of the `#KEY` key).
        pub fn key_count(&self) -> io::Result<u32> {
            let bytes = self.read_key("#KEY")?;
            Ok(be_u32_prefix(&bytes))
        }

        /// The 4-character name of the key at `index` (`0..key_count`). Lets us enumerate
        /// every key a given Mac exposes without hardcoding any.
        pub fn key_at_index(&self, index: u32) -> io::Result<String> {
            let mut input = SMCKeyData::new();
            input.data8 = SMC_CMD_READ_INDEX;
            input.data32 = index;
            let out = self.call(&input)?;
            if out.result != 0 {
                return Err(io::Error::other(format!(
                    "smc index result 0x{:02x}",
                    out.result
                )));
            }
            // The key name comes back big-endian in the `key` field; trailing NULs trimmed.
            Ok(key_name(out.key))
        }

        /// Read a key's raw bytes. Used to verify a write took, and (later) to detect
        /// which keys a given Mac model supports.
        pub fn read_key(&self, key: &str) -> io::Result<Vec<u8>> {
            let key_u32 = key_to_u32(key);
            let info = self.key_info(key_u32)?;

            let mut input = SMCKeyData::new();
            input.key = key_u32;
            input.data8 = SMC_CMD_READ_BYTES;
            input.key_info.data_size = info.data_size;
            let out = self.call(&input)?;
            if out.result != 0 {
                return Err(io::Error::other(format!(
                    "smc read result 0x{:02x}",
                    out.result
                )));
            }
            // Clamp to the 32-byte buffer so a bogus size can't over-read.
            let n = (info.data_size as usize).min(out.bytes.len());
            Ok(out.bytes.iter().copied().take(n).collect())
        }
    }

    impl Drop for SmcConn {
        fn drop(&mut self) {
            // SAFETY: self.0 is a live IOConnect, closed exactly once here.
            unsafe {
                IOServiceClose(self.0);
            }
        }
    }
}

/// Parse a space-separated lowercase-hex byte string (e.g. `"ff 00"`) into bytes.
fn parse_hex_bytes(s: &str) -> Result<Vec<u8>, std::num::ParseIntError> {
    s.split_whitespace()
        .map(|tok| u8::from_str_radix(tok, 16))
        .collect()
}

/// Format bytes as a space-separated lowercase-hex string (the `READ` reply / `WRITE` input form).
fn format_hex_bytes(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A parsed line of the stdin protocol. Parsing is kept separate from the `IOKit`
/// work (which lives in `main`) so the protocol can be unit-tested without SMC.
#[derive(Debug, PartialEq, Eq)]
enum Command<'a> {
    Ping,
    Write { key: &'a str, hex: &'a str },
    Read { key: &'a str },
    List,
    Unknown,
}

/// Parse one protocol line into a [`Command`]. Splits into at most 3 tokens so a
/// `WRITE`'s hex payload (which contains spaces) stays intact as the third field.
fn parse_command(line: &str) -> Command<'_> {
    let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
    match parts.as_slice() {
        ["PING"] => Command::Ping,
        ["WRITE", key, hex] => Command::Write { key, hex },
        ["READ", key] => Command::Read { key },
        ["LIST"] => Command::List,
        _ => Command::Unknown,
    }
}

fn main() {
    let conn = match smc::SmcConn::open() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("led-helper: failed to open SMC: {e}");
            std::process::exit(1);
        }
    };

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        match parse_command(&line) {
            Command::Ping => {
                let _ = writeln!(stdout, "PONG");
            }
            Command::Write { key, hex } => match parse_hex_bytes(hex) {
                Ok(data) => match conn.write_key(key, &data) {
                    Ok(()) => {
                        let _ = writeln!(stdout, "OK");
                    }
                    Err(e) => {
                        let _ = writeln!(stdout, "ERR {e}");
                    }
                },
                Err(e) => {
                    let _ = writeln!(stdout, "ERR bad hex: {e}");
                }
            },
            Command::Read { key } => match conn.read_key(key) {
                Ok(data) => {
                    let _ = writeln!(stdout, "OK {}", format_hex_bytes(&data));
                }
                Err(e) => {
                    let _ = writeln!(stdout, "ERR {e}");
                }
            },
            Command::List => match conn.key_count() {
                Ok(n) => {
                    let _ = writeln!(stdout, "OK count {n}");
                    for i in 0..n {
                        if let Ok(name) = conn.key_at_index(i) {
                            let _ = writeln!(stdout, "{name}");
                        }
                    }
                }
                Err(e) => {
                    let _ = writeln!(stdout, "ERR {e}");
                }
            },
            Command::Unknown => {
                let _ = writeln!(stdout, "ERR unknown command");
            }
        }
        // A flush error means the parent (the app) has gone away; stop rather than
        // spin writing into a broken pipe.
        if stdout.flush().is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::smc::{be_u32_prefix, key_name, key_to_u32};
    use super::{format_hex_bytes, parse_command, parse_hex_bytes, Command};

    #[test]
    fn formats_bytes_as_hex() {
        assert_eq!(format_hex_bytes(&[0xff, 0x00]), "ff 00");
        assert_eq!(format_hex_bytes(&[0x0a]), "0a");
        assert_eq!(format_hex_bytes(&[]), "");
    }

    #[test]
    fn hex_round_trips() {
        let bytes = [0x00, 0x7f, 0xab, 0xff];
        assert_eq!(
            parse_hex_bytes(&format_hex_bytes(&bytes)).as_deref(),
            Ok(bytes.as_slice())
        );
    }

    #[test]
    fn key_to_u32_packs_big_endian() {
        // A 4-char key packs to its four ASCII bytes, MSB first.
        assert_eq!(
            key_to_u32("ABCD"),
            u32::from_be_bytes([b'A', b'B', b'C', b'D'])
        );
    }

    #[test]
    fn key_to_u32_zero_pads_short_keys() {
        assert_eq!(key_to_u32("TC"), u32::from_be_bytes([b'T', b'C', 0, 0]));
        assert_eq!(key_to_u32(""), 0);
    }

    #[test]
    fn key_to_u32_truncates_overlong_keys() {
        // Only the first 4 bytes matter; extra characters are ignored, never panic.
        assert_eq!(key_to_u32("ABCDEF"), key_to_u32("ABCD"));
    }

    #[test]
    fn key_name_decodes_and_trims_nuls() {
        assert_eq!(
            key_name(u32::from_be_bytes([b'#', b'K', b'E', b'Y'])),
            "#KEY"
        );
        assert_eq!(key_name(u32::from_be_bytes([b'T', b'C', 0, 0])), "TC");
        assert_eq!(key_name(0), "");
    }

    #[test]
    fn key_name_round_trips_with_key_to_u32() {
        for k in ["#KEY", "ABCD", "F0Ac"] {
            assert_eq!(key_name(key_to_u32(k)), k);
        }
    }

    #[test]
    fn be_u32_prefix_reads_count() {
        // The `#KEY` value 00 00 08 03 observed on a real Mac == 2051 keys.
        assert_eq!(be_u32_prefix(&[0x00, 0x00, 0x08, 0x03]), 2051);
        assert_eq!(be_u32_prefix(&[0xff]), 0xff00_0000);
        assert_eq!(be_u32_prefix(&[]), 0);
    }

    #[test]
    fn parse_command_recognizes_each_verb() {
        assert_eq!(parse_command("PING"), Command::Ping);
        assert_eq!(parse_command("LIST"), Command::List);
        assert_eq!(parse_command("READ #KEY"), Command::Read { key: "#KEY" });
        assert_eq!(
            parse_command("WRITE ABCD ff 00"),
            Command::Write {
                key: "ABCD",
                hex: "ff 00"
            }
        );
    }

    #[test]
    fn parse_command_keeps_multibyte_hex_payload_intact() {
        // splitn(3) must not split the hex bytes apart.
        assert_eq!(
            parse_command("WRITE ABCD 01 02 03 04"),
            Command::Write {
                key: "ABCD",
                hex: "01 02 03 04"
            }
        );
    }

    #[test]
    fn parse_command_trims_and_rejects_unknown() {
        assert_eq!(parse_command("  PING  "), Command::Ping);
        assert_eq!(parse_command(""), Command::Unknown);
        assert_eq!(parse_command("BOGUS x y"), Command::Unknown);
        // READ needs exactly one argument; a stray extra token is rejected, not misread.
        assert_eq!(parse_command("READ #KEY extra"), Command::Unknown);
    }

    #[test]
    fn parses_two_bytes() {
        assert!(matches!(parse_hex_bytes("ff 00").as_deref(), Ok([255, 0])));
    }

    #[test]
    fn hex_is_case_insensitive() {
        // The app only ever sends lowercase, but the parser accepts either case.
        assert!(matches!(parse_hex_bytes("FF").as_deref(), Ok([255])));
        assert!(matches!(
            parse_hex_bytes("aB cD").as_deref(),
            Ok([0xab, 0xcd])
        ));
    }

    #[test]
    fn rejects_out_of_range_byte() {
        // Each token must fit in a u8; "1ff" (511) overflows and is rejected.
        assert!(parse_hex_bytes("1ff").is_err());
    }

    #[test]
    fn parses_single_byte() {
        assert!(matches!(parse_hex_bytes("7f").as_deref(), Ok([127])));
    }

    #[test]
    fn empty_is_ok_empty() {
        assert!(matches!(parse_hex_bytes("").as_deref(), Ok([])));
    }

    #[test]
    fn rejects_non_hex() {
        assert!(parse_hex_bytes("zz").is_err());
    }
}
