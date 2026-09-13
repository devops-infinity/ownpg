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
}

@test "lib.sh say() writes to stderr, not stdout" {
	run --separate-stderr bash -c 'source tools/lib.sh; say SUCCESS "hello"'
	[ "$status" -eq 0 ]
	[ -z "$output" ]
	[ "$stderr" = "[SUCCESS] hello" ]
}

@test "lib.sh die() prints FAILED to stderr and exits 1" {
	run --separate-stderr bash -c 'source tools/lib.sh; die "went wrong"'
	[ "$status" -eq 1 ]
	[ "$stderr" = "[FAILED] went wrong" ]
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
	echo "hello world" >clean.txt
	git add clean.txt
	run git -c core.hooksPath=.githooks commit -m "clean"
	[ "$status" -eq 0 ]
}

@test "pre-commit blocks a commit containing an obvious secret" {
	if ! command -v gitleaks >/dev/null 2>&1; then
		skip "gitleaks is not installed"
	fi
	generated_suffix="$(date +%s%N | shasum | cut -c1-30 | tr '[:lower:]' '[:upper:]')"
	printf 'AWS_SECRET_ACCESS_KEY="AKIA%s"\n' "$generated_suffix" >secret.txt
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

@test "verify.sh's comment detector catches a leading comment" {
	run bash -c 'printf "// a leading comment\n" | grep -nE "^\s*//|[[:space:]]//"'
	[ "$status" -eq 0 ]
}

@test "verify.sh's comment detector catches a trailing comment" {
	run bash -c 'printf "let x = 5; // trailing comment\n" | grep -nE "^\s*//|[[:space:]]//"'
	[ "$status" -eq 0 ]
}

@test "verify.sh's comment detector does not flag a URL in a string literal" {
	run bash -c 'printf "let url = \"https://example.com\";\n" | grep -nE "^\s*//|[[:space:]]//"'
	[ "$status" -eq 1 ]
}
