//! moof — the zig host for the V4 substrate (W5b rename: was `moof-zig`).
//!
//! produces a single binary `moof` at `zig-out/bin/`. consumes
//! V4 byte-tagged bytecode (produced by either the rust seed compiler
//! or, later, the OCaml seed compiler).
//!
//! the rust runtime is now `moof-rs` (build-time oracle / fallback only).
//! see `docs/superpowers/plans/2026-05-10-vm-V4-polyglot-substrate.md`
//! for the full migration arc.

const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const exe = b.addExecutable(.{
        .name = "moof",
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/main.zig"),
            .target = target,
            .optimize = optimize,
        }),
    });

    b.installArtifact(exe);

    const run_cmd = b.addRunArtifact(exe);
    run_cmd.step.dependOn(b.getInstallStep());
    if (b.args) |args| run_cmd.addArgs(args);

    const run_step = b.step("run", "run moof");
    run_step.dependOn(&run_cmd.step);

    // test step — collects unit tests from all src/*.zig files.
    const test_step = b.step("test", "run unit tests");

    const src_files = [_][]const u8{
        "src/intrinsics.zig",
        "src/test_vat_mode.zig",
    };

    for (src_files) |src| {
        const t = b.addTest(.{
            .root_module = b.createModule(.{
                .root_source_file = b.path(src),
                .target = target,
                .optimize = optimize,
            }),
        });
        const run_t = b.addRunArtifact(t);
        test_step.dependOn(&run_t.step);
    }
}
