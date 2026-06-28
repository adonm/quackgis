// SPDX-License-Identifier: Apache-2.0
//
//! Packaging helper: appends the 512-byte DuckDB extension metadata trailer
//! to a built shared object, producing a `.duckdb_extension` file that DuckDB
//! will accept via `LOAD` (with the `-unsigned` flag).
//!
//! The trailer layout (last 512 bytes of the file) is the same one
//! `quack-rs::append_metadata` produces:
//!
//! ```text
//!   Bytes   0– 31  Field 0: reserved
//!   Bytes  32– 63  Field 1: reserved
//!   Bytes  64– 95  Field 2: reserved
//!   Bytes  96–127  Field 3: ABI type         ("C_STRUCT")
//!   Bytes 128–159  Field 4: extension version ("v0.1.0")
//!   Bytes 160–191  Field 5: DuckDB C-API ver  ("v1.2.0")
//!   Bytes 192–223  Field 6: platform          ("linux_amd64")
//!   Bytes 224–255  Field 7: magic             ("4")
//!   Bytes 256–511  signature area (zero-filled = unsigned)
//! ```
//!
//! Usage:
//! ```sh
//! sedonadb-package <input.so> <output.duckdb_extension> [platform]
//! # then:
//! # duckdb -unsigned -c "LOAD '<output>'; SELECT ..."
//! ```

use std::env;
use std::fs;
use std::process::ExitCode;

const FIELD_SIZE: usize = 32;
const NUM_FIELDS: usize = 8;
const SIGNATURE_SIZE: usize = 256;
const METADATA_SIZE: usize = FIELD_SIZE * NUM_FIELDS + SIGNATURE_SIZE; // 512

fn make_field(s: &str) -> [u8; FIELD_SIZE] {
    let mut field = [0u8; FIELD_SIZE];
    let bytes = s.as_bytes();
    let n = bytes.len().min(FIELD_SIZE - 1);
    field[..n].copy_from_slice(&bytes[..n]);
    field
}

fn build_metadata(abi: &str, ext_version: &str, duckdb_version: &str, platform: &str) -> [u8; METADATA_SIZE] {
    let fields: [[u8; FIELD_SIZE]; NUM_FIELDS] = [
        make_field(""),            // 0: reserved
        make_field(""),            // 1: reserved
        make_field(""),            // 2: reserved
        make_field(abi),           // 3: ABI type
        make_field(ext_version),   // 4: extension version
        make_field(duckdb_version),// 5: DuckDB C-API version
        make_field(platform),      // 6: platform
        make_field("4"),           // 7: magic
    ];
    let mut block = [0u8; METADATA_SIZE];
    for (i, field) in fields.iter().enumerate() {
        block[i * FIELD_SIZE..(i + 1) * FIELD_SIZE].copy_from_slice(field);
    }
    // Bytes 256–511: signature area, zero-filled => unsigned extension.
    block
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 || args.len() > 4 {
        eprintln!(
            "usage: {} <input.so/.dylib/.dll> <output.duckdb_extension> [platform]",
            args.first().map(String::as_str).unwrap_or("sedonadb-package")
        );
        return ExitCode::from(2);
    }
    let input = &args[1];
    let output = &args[2];
    let platform = args.get(3).cloned().unwrap_or_else(detect_platform);

    let mut data = match fs::read(input) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot read {input}: {e}");
            return ExitCode::from(1);
        }
    };
    let metadata = build_metadata("C_STRUCT", "v0.1.0", "v1.2.0", &platform);
    data.extend_from_slice(&metadata);

    if let Err(e) = fs::write(output, &data) {
        eprintln!("error: cannot write {output}: {e}");
        return ExitCode::from(1);
    }

    println!(
        "packaged {} ({} bytes) + 512-byte trailer -> {} for platform {platform}",
        input,
        data.len() - METADATA_SIZE,
        output
    );
    ExitCode::SUCCESS
}

fn detect_platform() -> String {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;
    let os_str = match os {
        "linux" => "linux",
        "macos" => "osx",
        "windows" => "windows",
        other => other,
    };
    let arch_str = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{os_str}_{arch_str}")
}
