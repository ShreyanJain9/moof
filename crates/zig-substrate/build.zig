//! moof-zig — the zig host for the V4 substrate.
//!
//! produces a single binary `moof-zig` at `zig-out/bin/`. consumes
//! V4 byte-tagged bytecode (produced by either the rust seed compiler
//! or, later, the OCaml seed compiler).
//!
//! see `docs/superpowers/plans/2026-05-10-vm-V4-polyglot-substrate.md`
//! for the full migration arc.

const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const exe = b.addExecutable(.{
        .name = "moof-zig",
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

    const run_step = b.step("run", "run moof-zig");
    run_step.dependOn(&run_cmd.step);
}
