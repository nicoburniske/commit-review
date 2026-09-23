#!/usr/bin/env nu

const script = path self
const project = path self | path dirname

# run the stress app with optional profiling
def main [
    --profile # record call stacks with samply
    --load: path # open an existing profile instead of recording
    --save-only # save without opening the viewer
] {
    let load = if $load == null { null } else { $load | path expand }
    if ($env.COMMIT_REVIEW_PROFILING? | default "") != "1" {
        let environment = $project | path join target profiling env
        mkdir $environment
        cp ($project | path join flake.nix) ($project | path join flake.lock) $environment
        let flake = $"path:($environment)"
        let args = (if $load == null { [] } else { [--load $load] })
            | append (if $profile { [--profile] } else { [] })
            | append (if $save_only { [--save-only] } else { [] })
        with-env {COMMIT_REVIEW_PROFILING: "1"} {
            if $profile or $load != null {
                ^nix develop $flake --command nix shell --inputs-from $flake nixpkgs#samply --command nu $script ...$args
            } else {
                ^nix develop $flake --command nu $script ...$args
            }
        }
        exit $env.LAST_EXIT_CODE
    }

    if $load != null {
        ^samply load $load
        exit $env.LAST_EXIT_CODE
    }

    cd $project
    let target = $project | path join target profiling
    mkdir $target
    if $profile {
        let probe = ^samply record --save-only --output ($target | path join probe.json.gz) -- nu --no-config-file -c null | complete
        if $probe.exit_code != 0 {
            print --stderr $probe.stderr
            if ($probe.stderr | str contains "mmap failed") {
                print --stderr "Samply could not map its perf buffers. Increase the perf locked-memory allowance, then rerun:"
                print --stderr "  sudo sysctl kernel.perf_event_mlock_kb=2048"
            }
            exit $probe.exit_code
        }
    }
    $env.CARGO_PROFILE_RELEASE_DEBUG = "2"
    $env.CARGO_PROFILE_RELEASE_STRIP = "none"
    $env.CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO = "off"
    hide-env --ignore-errors CARGO_ENCODED_RUSTFLAGS
    $env.RUSTFLAGS = $"($env.RUSTFLAGS? | default '') -C force-frame-pointers=yes"
    ^cargo build --release --locked --target-dir $target
    if $env.LAST_EXIT_CODE != 0 { exit $env.LAST_EXIT_CODE }

    let recording = $project | path join target profiles (date now | format date "%Y%m%d-%H%M%S-%f")
    mkdir $recording
    let binary = $recording | path join commit-review
    let profile_path = $recording | path join profile.json.gz
    cp ($target | path join release commit-review) $binary
    let repo = $recording | path join repo
    let source = $project | path join .. blit | path expand
    let before = "f8bf72fe7f56be213a8faba038a0f76f25d335c6"
    let after = "75c283dea8219a927be270ccd6b6f054b6736138"
    ^git clone --quiet --no-hardlinks --no-checkout $source $repo
    if $env.LAST_EXIT_CODE != 0 { exit $env.LAST_EXIT_CODE }
    ^git -C $repo config core.hooksPath /dev/null
    ^git -C $repo checkout --quiet --detach $before
    if $env.LAST_EXIT_CODE != 0 { exit $env.LAST_EXIT_CODE }
    ^git -C $repo read-tree --reset -u $after
    if $env.LAST_EXIT_CODE != 0 { exit $env.LAST_EXIT_CODE }
    let profiler_args = if $save_only { [--save-only] } else { [] }
    print $"Stress repository: ($repo) — real Blit changes from f8bf72f to 75c283d."
    ^git -C $repo diff --cached --shortstat
    cd $repo
    if $profile {
        print $"Profile: ($profile_path)"
        print "Scroll for 15 to 30 seconds, then close the app."
        ^samply record --rate 1000 --output $profile_path ...$profiler_args -- $binary
    } else {
        ^$binary
    }
    exit $env.LAST_EXIT_CODE
}
