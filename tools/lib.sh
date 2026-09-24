#!/usr/bin/env bash

say() { printf '%s [%s] %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$1" "$2" >&2; }
die() {
	say FAILED "$1"
	exit 1
}

version_at_least() {
	local have="$1" want="$2" i left right
	local -a have_parts=() want_parts=()
	[[ "$have" =~ ^[0-9]+(\.[0-9]+)*$ && "$want" =~ ^[0-9]+(\.[0-9]+)*$ ]] || return 1
	IFS=. read -r -a have_parts <<<"$have"
	IFS=. read -r -a want_parts <<<"$want"
	for i in 0 1 2; do
		left="${have_parts[i]:-0}"
		right="${want_parts[i]:-0}"
		if ((10#$left > 10#$right)); then
			return 0
		fi
		if ((10#$left < 10#$right)); then
			return 1
		fi
	done
	return 0
}

require_bash() {
	local want="$1" candidate found
	shift
	version_at_least "${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}" "$want" && return 0
	if [[ $# -gt 0 && -z "${OWNPG_BASH_REEXEC:-}" ]]; then
		for candidate in /opt/homebrew/bin/bash /usr/local/bin/bash /home/linuxbrew/.linuxbrew/bin/bash; do
			[[ -x "$candidate" && "$candidate" != "$BASH" ]] || continue
			found="$("$candidate" -c "printf '%s.%s' \"\${BASH_VERSINFO[0]}\" \"\${BASH_VERSINFO[1]}\"")" || continue
			if version_at_least "$found" "$want"; then
				OWNPG_BASH_REEXEC=1 exec "$candidate" "$@"
			fi
		done
	fi
	die "bash $want or newer is required, but $BASH is bash $BASH_VERSION; install a newer bash (for example: brew install bash)"
}

live_tests_enabled() {
	local dsn="${OWNPG_TEST_DSN:-}"
	if [[ -n "${dsn//[[:space:]]/}" ]]; then
		return 0
	fi
	if [[ "${OWNPG_SKIP_LIVE_TESTS:-}" == "1" ]]; then
		return 1
	fi
	die "OWNPG_TEST_DSN is not set, so the live database tests would pass without running; set it to a libpq connection string (postgresql://... or host=... user=... dbname=...) for a role that can create databases, or set OWNPG_SKIP_LIVE_TESTS=1 to skip the live tests"
}

existing_files_from_stdin() {
	local file
	while IFS= read -r -d '' file; do
		[[ -f "$file" ]] && printf '%s\0' "$file"
	done
	return 0
}

rust_comment_lines() {
	local file
	local -a files=()
	while IFS= read -r -d '' file; do
		files+=("$file")
	done < <(existing_files_from_stdin)
	[[ ${#files[@]} -gt 0 ]] || return 0
	grep -HnE '^\s*//|[[:space:]]//|(^|[[:space:]])/\*' -- "${files[@]}" || true
}

hash_comment_lines() {
	local file
	local -a files=()
	while IFS= read -r -d '' file; do
		files+=("$file")
	done < <(existing_files_from_stdin)
	[[ ${#files[@]} -gt 0 ]] || return 0
	awk 'FNR == 1 && /^#!/ { next } /^[[:space:]]*#/ { printf "%s:%d:%s\n", FILENAME, FNR, $0 }' "${files[@]}"
}

acquire_pid_lock() {
	local dir="$1" holder=""
	if ! mkdir -- "$dir" 2>/dev/null; then
		[[ -f "$dir/pid" ]] || return 1
		IFS= read -r holder <"$dir/pid" || true
		[[ "$holder" =~ ^[0-9]+$ ]] || return 1
		if kill -0 "$holder" 2>/dev/null; then
			return 1
		fi
		say WARNING "clearing a stale lock left by process $holder, which is no longer running: $dir"
		rm -f -- "$dir/pid"
		rmdir -- "$dir" 2>/dev/null || return 1
		mkdir -- "$dir" 2>/dev/null || return 1
	fi
	if ! printf '%s\n' "$$" >"$dir/pid"; then
		rmdir -- "$dir" 2>/dev/null || true
		return 1
	fi
}

release_pid_lock() {
	local dir="$1" holder=""
	[[ -d "$dir" ]] || return 0
	if [[ -f "$dir/pid" ]]; then
		IFS= read -r holder <"$dir/pid" || true
		[[ "$holder" == "$$" ]] || return 0
		rm -f -- "$dir/pid"
	fi
	rmdir -- "$dir" 2>/dev/null || true
}

sweep_stale_install_temps() {
	local dir="$1" bin="$2"
	[[ -d "$dir" ]] || return 0
	find "$dir" -maxdepth 1 -type f \( -name ".$bin.??????" -o -name ".$bin.exe.??????" \) -mmin +1440 -delete
}

json_version_filter() {
	local filter="." path
	for path in "$@"; do
		filter+=" | ($path) = \$version"
	done
	printf '%s\n' "$filter"
}

json_versions_match() {
	local file="$1" version="$2" filter wanted current
	shift 2
	filter="$(json_version_filter "$@")"
	wanted="$(jq -S --arg version "$version" "$filter" "$file")" || return 1
	current="$(jq -S . "$file")" || return 1
	[[ "$wanted" == "$current" ]]
}

set_json_versions() {
	local file="$1" version="$2" out="$3" filter wanted
	shift 3
	filter="$(json_version_filter "$@")"
	wanted="$(jq -S --arg version "$version" "$filter" "$file")" || return 1
	awk -v new="$version" '
		match($0, /"version"[[:space:]]*:[[:space:]]*"/) {
			head = substr($0, 1, RSTART + RLENGTH - 1)
			tail = substr($0, RSTART + RLENGTH)
			sub(/^[^"]*/, "", tail)
			print head new tail
			next
		}
		{ print }
	' "$file" >"$out" || return 1
	[[ "$(jq -S . "$out" 2>/dev/null)" == "$wanted" ]]
}
