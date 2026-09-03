#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
INSTALLER="$SCRIPT_DIR/../install.sh"
TEST_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/zerobeat-installer-test.XXXXXX")
trap 'rm -rf "$TEST_ROOT"' 0 1 2 3 15

fail() {
    printf 'installer distro test: %s\n' "$*" >&2
    exit 1
}

command -v curl >/dev/null 2>&1 || fail 'curl is required'
command -v readelf >/dev/null 2>&1 || fail 'readelf is required'
command -v sha256sum >/dev/null 2>&1 || fail 'sha256sum is required'
command -v tar >/dev/null 2>&1 || fail 'tar is required'

PAYLOAD_DIR="$TEST_ROOT/payload"
mkdir -p "$PAYLOAD_DIR"
cp /bin/true "$PAYLOAD_DIR/zerobeat-cli"
cp /bin/true "$PAYLOAD_DIR/zerobeatd"
printf 'fixture\n' > "$PAYLOAD_DIR/README.md"
printf 'fixture\n' > "$PAYLOAD_DIR/LICENSE"

make_fixture() {
    fixture_dir=$1
    archive=$2
    mkdir -p "$fixture_dir"
    tar -czf "$fixture_dir/$archive" -C "$PAYLOAD_DIR" \
        zerobeat-cli zerobeatd README.md LICENSE
    sha256sum "$fixture_dir/$archive" > "$fixture_dir/$archive.sha256"
}

FIXTURE_DIR12="$TEST_ROOT/releases12"
FIXTURE_DIR13="$TEST_ROOT/releases13"
FIXTURE_DIR12_AARCH64="$TEST_ROOT/releases12-aarch64"
FIXTURE_DIR13_AARCH64="$TEST_ROOT/releases13-aarch64"
FIXTURE_DIR_UBUNTU24_AARCH64="$TEST_ROOT/releases-ubuntu24-aarch64"
make_fixture "$FIXTURE_DIR12" zerobeat-linux-debian12-x86_64.tar.gz
make_fixture "$FIXTURE_DIR13" zerobeat-linux-debian13-x86_64.tar.gz
make_fixture "$FIXTURE_DIR12_AARCH64" zerobeat-linux-debian12-aarch64.tar.gz
make_fixture "$FIXTURE_DIR13_AARCH64" zerobeat-linux-debian13-aarch64.tar.gz
make_fixture "$FIXTURE_DIR_UBUNTU24_AARCH64" zerobeat-linux-ubuntu24-aarch64.tar.gz

run_install() {
    version=$1
    prefix=$2
    os_release=$3
    fixture_dir=$4
    output=$5
    uname_m=$6
    ZEROBEAT_ALLOW_LOCAL_TEST=1 \
    ZEROBEAT_OS_RELEASE="$os_release" \
    ZEROBEAT_UNAME_M="$uname_m" \
    ZEROBEAT_RELEASE_BASE_URL="file://$fixture_dir" \
    PREFIX="$prefix" \
    VERSION="$version" \
    sh "$INSTALLER" > "$output" 2>&1
}

write_os_release() {
    id=$1
    version=$2
    path=$3
    printf 'ID=%s\nVERSION_ID=%s\n' "$id" "$version" > "$path"
}

DEBIAN12_OS="$TEST_ROOT/debian12-os-release"
DEBIAN13_OS="$TEST_ROOT/debian13-os-release"
DEBIAN11_OS="$TEST_ROOT/debian11-os-release"
UBUNTU24_OS="$TEST_ROOT/ubuntu24-os-release"
write_os_release debian 12 "$DEBIAN12_OS"
write_os_release debian 13 "$DEBIAN13_OS"
write_os_release debian 11 "$DEBIAN11_OS"
write_os_release ubuntu 24.04 "$UBUNTU24_OS"

PREFIX12="$TEST_ROOT/prefix12"
run_install latest "$PREFIX12" "$DEBIAN12_OS" "$FIXTURE_DIR12" "$TEST_ROOT/debian12.out" x86_64 || {
    cat "$TEST_ROOT/debian12.out" >&2
    fail 'Debian 12 installation was rejected'
}
[ -x "$PREFIX12/bin/zerobeat-cli" ] || fail 'Debian 12 did not install zerobeat-cli'

PREFIX13="$TEST_ROOT/prefix13"
run_install latest "$PREFIX13" "$DEBIAN13_OS" "$FIXTURE_DIR13" "$TEST_ROOT/debian13.out" x86_64 || {
    cat "$TEST_ROOT/debian13.out" >&2
    fail 'Debian 13 installation was rejected'
}
[ -x "$PREFIX13/bin/zerobeat-cli" ] || fail 'Debian 13 did not install zerobeat-cli'

if run_install latest "$TEST_ROOT/prefix11" "$DEBIAN11_OS" "$FIXTURE_DIR12" "$TEST_ROOT/debian11.out" x86_64; then
    cat "$TEST_ROOT/debian11.out" >&2
    fail 'unsupported Debian 11 installation was accepted'
fi
grep -F 'unsupported Debian version: 11' "$TEST_ROOT/debian11.out" >/dev/null 2>&1 || {
    cat "$TEST_ROOT/debian11.out" >&2
    fail 'Debian 11 rejection did not identify the unsupported version'
}

if run_install latest "$TEST_ROOT/prefix12-aarch64" "$DEBIAN12_OS" "$FIXTURE_DIR12_AARCH64" "$TEST_ROOT/debian12-aarch64.out" aarch64; then
    cat "$TEST_ROOT/debian12-aarch64.out" >&2
    fail 'aarch64 Debian 12 fixture unexpectedly passed x86 ELF validation'
fi
grep -F 'zerobeat-cli is not an AArch64 binary' "$TEST_ROOT/debian12-aarch64.out" >/dev/null 2>&1 || {
    cat "$TEST_ROOT/debian12-aarch64.out" >&2
    fail 'aarch64 Debian 12 did not select the aarch64 archive or validate its ELF architecture'
}

if run_install latest "$TEST_ROOT/prefix13-aarch64" "$DEBIAN13_OS" "$FIXTURE_DIR13_AARCH64" "$TEST_ROOT/debian13-aarch64.out" aarch64; then
    cat "$TEST_ROOT/debian13-aarch64.out" >&2
    fail 'aarch64 Debian 13 fixture unexpectedly passed x86 ELF validation'
fi
grep -F 'zerobeat-cli is not an AArch64 binary' "$TEST_ROOT/debian13-aarch64.out" >/dev/null 2>&1 || {
    cat "$TEST_ROOT/debian13-aarch64.out" >&2
    fail 'aarch64 Debian 13 did not select the aarch64 archive or validate its ELF architecture'
}

if run_install latest "$TEST_ROOT/prefix-ubuntu24-aarch64" "$UBUNTU24_OS" "$FIXTURE_DIR_UBUNTU24_AARCH64" "$TEST_ROOT/ubuntu24-aarch64.out" aarch64; then
    cat "$TEST_ROOT/ubuntu24-aarch64.out" >&2
    fail 'aarch64 Ubuntu 24.04 fixture unexpectedly passed x86 ELF validation'
fi
grep -F 'zerobeat-cli is not an AArch64 binary' "$TEST_ROOT/ubuntu24-aarch64.out" >/dev/null 2>&1 || {
    cat "$TEST_ROOT/ubuntu24-aarch64.out" >&2
    fail 'aarch64 Ubuntu 24.04 did not select the aarch64 archive or validate its ELF architecture'
}

if run_install latest "$TEST_ROOT/prefix-unsupported-arch" "$DEBIAN12_OS" "$FIXTURE_DIR12" "$TEST_ROOT/unsupported-arch.out" armv7l; then
    cat "$TEST_ROOT/unsupported-arch.out" >&2
    fail 'unsupported architecture was accepted'
fi
grep -F 'only Linux x86_64 or aarch64 is supported' "$TEST_ROOT/unsupported-arch.out" >/dev/null 2>&1 || {
    cat "$TEST_ROOT/unsupported-arch.out" >&2
    fail 'unsupported architecture rejection did not identify the supported architectures'
}

printf 'installer distro test: x86_64 Debian 12/13 accepted; aarch64 Debian 12/13 and Ubuntu 24.04 selected; Debian 11 and armv7l rejected\n'
