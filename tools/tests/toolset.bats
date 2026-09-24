#!/usr/bin/env bats

bats_require_minimum_version 1.5.0

setup() {
	REPO_ROOT="$(cd -- "$BATS_TEST_DIRNAME/../.." && pwd)"
	TEST_REPO="$BATS_TEST_TMPDIR/repo"
	mkdir -p "$TEST_REPO/tools" "$TEST_REPO/.githooks"
	cp "$REPO_ROOT/tools/lib.sh" "$TEST_REPO/tools/lib.sh"
	cp "$REPO_ROOT/tools/install-hooks.sh" "$TEST_REPO/tools/install-hooks.sh"
	cp "$REPO_ROOT/.githooks/pre-commit" "$TEST_REPO/.githooks/pre-commit"
	cp "$REPO_ROOT/.githooks/pre-push" "$TEST_REPO/.githooks/pre-push"
	chmod +x "$TEST_REPO/tools/install-hooks.sh" "$TEST_REPO/.githooks/pre-commit" "$TEST_REPO/.githooks/pre-push"
	cd "$TEST_REPO" || exit
	git init -q
	git config user.email "test@example.com"
	git config user.name "test"
	TIMESTAMP='[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z'
}

@test "lib.sh say() writes a UTC timestamp, the level, and the message to stderr, not stdout" {
	run --separate-stderr bash -c 'source tools/lib.sh; say SUCCESS "hello"'
	[ "$status" -eq 0 ]
	[ -z "$output" ]
	[[ "$stderr" =~ ^${TIMESTAMP}\ \[SUCCESS\]\ hello$ ]]
}

@test "lib.sh die() prints a timestamped FAILED line to stderr and exits 1" {
	run --separate-stderr bash -c 'source tools/lib.sh; die "went wrong"'
	[ "$status" -eq 1 ]
	[[ "$stderr" =~ ^${TIMESTAMP}\ \[FAILED\]\ went\ wrong$ ]]
}

@test "version_at_least accepts equal and newer versions and rejects older ones" {
	run bash -c 'source tools/lib.sh; version_at_least 1.8.2 1.8.2 && version_at_least 8.22.0 8.7.1 && version_at_least 5.3 4.4 && version_at_least 1.10 1.9.9'
	[ "$status" -eq 0 ]
	run bash -c 'source tools/lib.sh; version_at_least 8.7.1 8.22.0'
	[ "$status" -eq 1 ]
	run bash -c 'source tools/lib.sh; version_at_least 1.7 1.8.2'
	[ "$status" -eq 1 ]
	run bash -c 'source tools/lib.sh; version_at_least 1.8.x 1.8.2'
	[ "$status" -eq 1 ]
}

@test "require_bash passes on this bash and names the floor when bash is too old" {
	run bash -c 'source tools/lib.sh; require_bash 4.4'
	[ "$status" -eq 0 ]
	run --separate-stderr bash -c 'source tools/lib.sh; require_bash 99.0'
	[ "$status" -eq 1 ]
	[[ "$stderr" == *"bash 99.0 or newer is required"* ]]
	run --separate-stderr bash -c 'source tools/lib.sh; require_bash 99.0 /dev/null'
	[ "$status" -eq 1 ]
	[[ "$stderr" == *"brew install bash"* ]]
}

@test "require_bash hands an old bash over to a newer one when one is installed" {
	/bin/bash -c '[[ ${BASH_VERSINFO[0]} -lt 4 ]]' || skip "/bin/bash is already new enough"
	local newer="" candidate
	for candidate in /opt/homebrew/bin/bash /usr/local/bin/bash; do
		if [[ -x "$candidate" ]]; then
			newer="$candidate"
			break
		fi
	done
	[[ -n "$newer" ]] || skip "no newer bash is installed"
	local script="$BATS_TEST_TMPDIR/probe.sh"
	printf '%s\n' '#!/bin/bash' 'source "$1/tools/lib.sh"' 'require_bash 4.4 "$0" "$@"' 'printf "%s.%s\n" "${BASH_VERSINFO[0]}" "${BASH_VERSINFO[1]}"' >"$script"
	run /bin/bash "$script" "$PWD"
	[ "$status" -eq 0 ]
	[[ "$output" =~ ^([5-9]|4\.[4-9]) ]]
}

@test "live_tests_enabled succeeds when OWNPG_TEST_DSN is set" {
	run env OWNPG_TEST_DSN="postgresql://tester@127.0.0.1:5432/postgres" bash -c 'source tools/lib.sh; live_tests_enabled'
	[ "$status" -eq 0 ]
}

@test "live_tests_enabled refuses to run without OWNPG_TEST_DSN" {
	run --separate-stderr env -u OWNPG_TEST_DSN -u OWNPG_SKIP_LIVE_TESTS bash -c 'source tools/lib.sh; live_tests_enabled; echo "continued"'
	[ "$status" -eq 1 ]
	[ -z "$output" ]
	[[ "$stderr" == *"OWNPG_TEST_DSN is not set"* ]]
	[[ "$stderr" == *"OWNPG_SKIP_LIVE_TESTS=1"* ]]
}

@test "live_tests_enabled treats a blank OWNPG_TEST_DSN as unset" {
	run --separate-stderr env -u OWNPG_SKIP_LIVE_TESTS OWNPG_TEST_DSN="   " bash -c 'source tools/lib.sh; live_tests_enabled; echo "continued"'
	[ "$status" -eq 1 ]
	[[ "$stderr" == *"OWNPG_TEST_DSN is not set"* ]]
}

@test "live_tests_enabled reports a skip without stopping when OWNPG_SKIP_LIVE_TESTS=1" {
	run env -u OWNPG_TEST_DSN OWNPG_SKIP_LIVE_TESTS=1 bash -c 'source tools/lib.sh; live_tests_enabled || echo "skipped"'
	[ "$status" -eq 0 ]
	[ "$output" = "skipped" ]
}

@test "acquire_pid_lock takes a free lock and records its own process id" {
	run bash -c 'source tools/lib.sh; acquire_pid_lock release.lock && [ "$(cat release.lock/pid)" = "$$" ]'
	[ "$status" -eq 0 ]
}

@test "acquire_pid_lock refuses a lock held by a running process" {
	mkdir release.lock
	printf '%s\n' "$$" >release.lock/pid
	run bash -c 'source tools/lib.sh; acquire_pid_lock release.lock'
	[ "$status" -eq 1 ]
	[ "$(cat release.lock/pid)" = "$$" ]
}

@test "acquire_pid_lock clears a lock left by a process that is gone" {
	bash -c 'exit 0' &
	gone=$!
	wait "$gone" || true
	mkdir release.lock
	printf '%s\n' "$gone" >release.lock/pid
	run --separate-stderr bash -c 'source tools/lib.sh; acquire_pid_lock release.lock && [ "$(cat release.lock/pid)" = "$$" ]'
	[ "$status" -eq 0 ]
	[[ "$stderr" == *"clearing a stale lock left by process $gone"* ]]
}

@test "acquire_pid_lock refuses a lock directory with no process id in it" {
	mkdir release.lock
	run bash -c 'source tools/lib.sh; acquire_pid_lock release.lock'
	[ "$status" -eq 1 ]
	[ -d release.lock ]
}

@test "release_pid_lock removes its own lock and leaves another process's lock alone" {
	run bash -c 'source tools/lib.sh; acquire_pid_lock release.lock && release_pid_lock release.lock'
	[ "$status" -eq 0 ]
	[ ! -e release.lock ]
	mkdir release.lock
	printf '%s\n' "$$" >release.lock/pid
	run bash -c 'source tools/lib.sh; release_pid_lock release.lock'
	[ "$status" -eq 0 ]
	[ "$(cat release.lock/pid)" = "$$" ]
}

@test "sweep_stale_install_temps deletes day-old install temp files and keeps everything else" {
	mkdir bin
	touch -t 202001010000 bin/.ownpg.abc123 bin/.ownpg.exe.def456 bin/.ownpg.toolong7 bin/ownpg
	touch bin/.ownpg.fresh1
	run bash -c 'source tools/lib.sh; sweep_stale_install_temps bin ownpg'
	[ "$status" -eq 0 ]
	[ ! -e bin/.ownpg.abc123 ]
	[ ! -e bin/.ownpg.exe.def456 ]
	[ -e bin/.ownpg.fresh1 ]
	[ -e bin/.ownpg.toolong7 ]
	[ -e bin/ownpg ]
}

@test "sweep_stale_install_temps does nothing when the install directory does not exist" {
	run bash -c 'source tools/lib.sh; sweep_stale_install_temps missing-dir ownpg'
	[ "$status" -eq 0 ]
}

@test "install-hooks.sh points core.hooksPath at .githooks and makes the hooks executable" {
	chmod -x .githooks/pre-commit .githooks/pre-push
	printf '#!/usr/bin/env bash\n' >tools/verify.sh
	chmod +x tools/verify.sh
	run tools/install-hooks.sh
	[ "$status" -eq 0 ]
	run git config --get core.hooksPath
	[ "$output" = ".githooks" ]
	[ -x .githooks/pre-commit ]
	[ -x .githooks/pre-push ]
}

@test "pre-commit allows a commit with no secret" {
	if ! command -v gitleaks >/dev/null 2>&1; then
		skip "gitleaks is not installed"
	fi
	echo "hello world" >clean.txt
	git add clean.txt
	run git -c core.hooksPath=.githooks commit -m "clean"
	[ "$status" -eq 0 ]
}

@test "pre-commit blocks a commit containing an obvious secret" {
	if ! command -v gitleaks >/dev/null 2>&1; then
		skip "gitleaks is not installed"
	fi
	generated_suffix="$(head -c 4096 /dev/urandom | LC_ALL=C tr -dc 'A-Z2-7' | cut -c1-16)"
	[ "${#generated_suffix}" -eq 16 ]
	printf 'AWS_ACCESS_KEY_ID=AKIA%s\n' "$generated_suffix" >secret.txt
	git add secret.txt
	run git -c core.hooksPath=.githooks commit -m "secret"
	[ "$status" -ne 0 ]
}

@test "pre-push refuses to run when tools/verify.sh is missing" {
	run .githooks/pre-push
	[ "$status" -eq 1 ]
	[[ "$output" == *"tools/verify.sh is missing or not executable"* ]]
}

@test "pre-push propagates the exit code of tools/verify.sh" {
	printf '#!/usr/bin/env bash\nexit 7\n' >tools/verify.sh
	chmod +x tools/verify.sh
	run .githooks/pre-push
	[ "$status" -eq 7 ]
}

@test "pre-push exits 0 and reports success when tools/verify.sh passes" {
	printf '#!/usr/bin/env bash\nexit 0\n' >tools/verify.sh
	chmod +x tools/verify.sh
	run --separate-stderr .githooks/pre-push
	[ "$status" -eq 0 ]
	[[ "$stderr" == *"pre-push checks passed"* ]]
}

@test "verify.sh's Rust comment detector catches a leading comment" {
	printf '// a leading comment\n' >sample.rs
	run bash -c 'source tools/lib.sh; printf "sample.rs\0" | rust_comment_lines'
	[ "$output" = "sample.rs:1:// a leading comment" ]
}

@test "verify.sh's Rust comment detector catches a trailing comment" {
	printf 'let x = 5; // trailing comment\n' >sample.rs
	run bash -c 'source tools/lib.sh; printf "sample.rs\0" | rust_comment_lines'
	[ "$output" = "sample.rs:1:let x = 5; // trailing comment" ]
}

@test "verify.sh's Rust comment detector catches a block comment" {
	printf 'let x = 5;\n/* a block comment */\nlet y = 6; /* trailing block */\n' >sample.rs
	run bash -c 'source tools/lib.sh; printf "sample.rs\0" | rust_comment_lines'
	[ "${lines[0]}" = "sample.rs:2:/* a block comment */" ]
	[ "${lines[1]}" = "sample.rs:3:let y = 6; /* trailing block */" ]
	[ "${#lines[@]}" -eq 2 ]
}

@test "verify.sh's Rust comment detector does not flag a URL or a glob in a string literal" {
	printf 'let url = "https://example.com";\nlet glob = "/*";\n' >sample.rs
	run bash -c 'source tools/lib.sh; printf "sample.rs\0" | rust_comment_lines'
	[ "$status" -eq 0 ]
	[ -z "$output" ]
}

@test "verify.sh's hash comment detector flags a comment line and skips the shebang" {
	printf '#!/usr/bin/env bash\necho hi\n  # an indented comment\nx=1\n' >sample.sh
	printf '# a first-line comment\nkey = 1\n' >sample.toml
	run bash -c 'source tools/lib.sh; printf "sample.sh\0sample.toml\0" | hash_comment_lines'
	[ "${lines[0]}" = "sample.sh:3:  # an indented comment" ]
	[ "${lines[1]}" = "sample.toml:1:# a first-line comment" ]
	[ "${#lines[@]}" -eq 2 ]
}

@test "verify.sh's hash comment detector skips a tracked file that is gone from the working tree" {
	printf 'key = 1\n' >sample.toml
	run bash -c 'source tools/lib.sh; printf "gone.toml\0sample.toml\0" | hash_comment_lines'
	[ "$status" -eq 0 ]
	[ -z "$output" ]
}

@test "release.sh --resume needs the exact version it finishes" {
	cp "$REPO_ROOT/tools/release.sh" "$TEST_REPO/tools/release.sh"
	chmod +x "$TEST_REPO/tools/release.sh"
	run --separate-stderr tools/release.sh --resume --patch
	[ "$status" -eq 1 ]
	[[ "$stderr" == *"--resume needs the version being finished"* ]]
	run --separate-stderr tools/release.sh --resume --dry-run --version 1.2.3
	[ "$status" -eq 1 ]
	[[ "$stderr" == *"cannot be combined with --dry-run"* ]]
	[ ! -e .git/release.lock ]
}

@test "release.sh documents --resume in its usage text" {
	cp "$REPO_ROOT/tools/release.sh" "$TEST_REPO/tools/release.sh"
	chmod +x "$TEST_REPO/tools/release.sh"
	run tools/release.sh --help
	[ "$status" -eq 0 ]
	[[ "$output" == *"--resume --version X.Y.Z"* ]]
}
