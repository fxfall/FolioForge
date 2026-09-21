// swift-tools-version: 6.4

import Foundation
import PackageDescription

let folioFFIArchive = ProcessInfo.processInfo.environment["FOLIOFORGE_FFI_ARCHIVE"]
    ?? "../../target/aarch64-apple-darwin/release/libfolio_ffi.a"

let package = Package(
    name: "FolioForge",
    platforms: [
        .macOS(.v27),
    ],
    products: [
        .executable(name: "FolioForge", targets: ["FolioForge"]),
    ],
    targets: [
        .executableTarget(
            name: "FolioForge",
            path: ".",
            exclude: ["Package.swift", "README.md"],
            resources: [
                .process("Resources"),
            ],
            linkerSettings: [
                // Build the Rust FFI for macOS 27 first with:
                // CFLAGS_aarch64_apple_darwin='-mmacosx-version-min=27.0' RUSTC_WRAPPER=packaging/rustc_macos_target_wrapper.sh cargo build --target aarch64-apple-darwin --release -p folio-ffi
                // The static archive keeps the command-line-toolchain build
                // independent of the host's dynamic linker layout. The
                // release app is intentionally unsigned.
                .unsafeFlags([folioFFIArchive]),
                // `zip`'s bzip2 and xz backends are native dependencies. Cargo
                // records those link flags for Rust binaries, but a Swift
                // executable that consumes the generated static archive must
                // repeat them at its final link boundary.
                .linkedLibrary("bz2"),
                .linkedLibrary("lzma"),
            ]
        ),
    ]
)
