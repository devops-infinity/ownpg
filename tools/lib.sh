#!/usr/bin/env bash

say() { printf '[%s] %s\n' "$1" "$2" >&2; }
die() {
	say FAILED "$1"
	exit 1
}
