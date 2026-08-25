#!/bin/sh
set -eu
LC_ALL=C
export LC_ALL

REPOSITORY=${ZEROBEAT_REPOSITORY:-Finsiii/ZeroBeat-CLI}
VERSION=${VERSION:-latest}
HOME_DIR=${HOME:-}
PREFIX=${PREFIX:-}
BINDIR=''
SHARE_DIR=''
MANIFEST_DIR=''
MANIFEST=''
ACTION=install

die() {
    printf 'install.sh: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Usage: ./install.sh [--uninstall]

Install the latest stable ZeroBeat Linux x86_64 release, or set VERSION=vX.Y.Z.
Set PREFIX to choose a destination (default: $HOME/.local). The installer never
uses sudo and never removes user data.

Examples:
  ./install.sh
  VERSION=v0.1.3 ./install.sh
  PREFIX=/usr/local VERSION=v0.1.3 ./install.sh
  ./install.sh --uninstall
EOF
}

case "${1:-}" in
    --help|-h)
        usage
        exit 0
        ;;
    --uninstall)
        [ "$#" -eq 1 ] || die '--uninstall does not accept additional arguments'
        ACTION=uninstall
        ;;
    '')
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac

resolve_prefix() {
    if [ -z "$PREFIX" ]; then
        [ -n "$HOME_DIR" ] || die 'HOME is not set; set HOME or PREFIX explicitly'
        PREFIX="$HOME_DIR/.local"
    fi
    case "$PREFIX" in
        /*) ;;
        *) die 'PREFIX must be an absolute path' ;;
    esac
    BINDIR="$PREFIX/bin"
    SHARE_DIR="$PREFIX/share"
    MANIFEST_DIR="$SHARE_DIR/zerobeat-cli"
    MANIFEST="$MANIFEST_DIR/install-manifest"
}

resolve_prefix
[ "$(uname -s)" = Linux ] || die 'only Linux is supported'
[ "$(uname -m)" = x86_64 ] || die 'only Linux x86_64 is supported'

reject_path_components() {
    path=$1
    current=''
    old_ifs=$IFS
    set -f
    IFS=/
    for component in $path; do
        [ -n "$component" ] || continue
        current="$current/$component"
        [ ! -L "$current" ] || die "refusing symlink path component: $current"
    done
    IFS=$old_ifs
    set +f
}

reject_symlink() {
    path=$1
    if [ -L "$path" ]; then
        die "refusing symlink path: $path"
    fi
}

recheck_paths() {
    reject_path_components "$PREFIX"
    reject_path_components "$BINDIR"
    reject_path_components "$SHARE_DIR"
    reject_path_components "$MANIFEST_DIR"
    reject_symlink "$PREFIX"
    reject_symlink "$BINDIR"
    reject_symlink "$SHARE_DIR"
    reject_symlink "$MANIFEST_DIR"
    reject_symlink "$MANIFEST"
    reject_symlink "$BINDIR/zerobeat-cli"
    reject_symlink "$BINDIR/zerobeatd"
}

ensure_owned() {
    path=$1
    owner=$(stat -c '%u' "$path" 2>/dev/null) || return 1
    [ "$owner" = "$(id -u)" ]
}

valid_digest() {
    digest=$1
    case "$digest" in
        ''|*[!0-9A-Fa-f]*) return 1 ;;
    esac
    [ "${#digest}" -eq 64 ]
}

read_manifest() {
    [ -f "$MANIFEST" ] || return 1
    reject_symlink "$MANIFEST"
    ensure_owned "$MANIFEST" || return 1
    MANIFEST_ZEROBEAT=$(awk '$1 == "zerobeat-cli" && NF == 2 { print $2 }' "$MANIFEST")
    MANIFEST_DAEMON=$(awk '$1 == "zerobeatd" && NF == 2 { print $2 }' "$MANIFEST")
    valid_digest "$MANIFEST_ZEROBEAT" || return 1
    valid_digest "$MANIFEST_DAEMON" || return 1
    [ "$(awk 'NF != 2 || ($1 != "zerobeat-cli" && $1 != "zerobeatd") { bad=1 } END { print bad + 0 }' "$MANIFEST")" -eq 0 ] ||
        return 1
    [ "$(grep -c -E '^zerobeat-cli ' "$MANIFEST")" -eq 1 ] || return 1
    [ "$(grep -c -E '^zerobeatd ' "$MANIFEST")" -eq 1 ] || return 1
}

verify_installed_pair() {
    read_manifest || return 1
    for binary in zerobeat-cli zerobeatd; do
        path="$BINDIR/$binary"
        [ -f "$path" ] || return 1
        reject_symlink "$path"
        ensure_owned "$path" || return 1
    done
    [ "$(sha256sum "$BINDIR/zerobeat-cli" | awk '{print $1}')" = "$MANIFEST_ZEROBEAT" ] || return 1
    [ "$(sha256sum "$BINDIR/zerobeatd" | awk '{print $1}')" = "$MANIFEST_DAEMON" ] || return 1
}

command -v sha256sum >/dev/null 2>&1 || die 'sha256sum is required'
command -v awk >/dev/null 2>&1 || die 'awk is required'
command -v grep >/dev/null 2>&1 || die 'grep is required'
command -v sed >/dev/null 2>&1 || die 'sed is required'
command -v stat >/dev/null 2>&1 || die 'stat is required'
command -v id >/dev/null 2>&1 || die 'id is required'
command -v mkdir >/dev/null 2>&1 || die 'mkdir is required'
command -v rmdir >/dev/null 2>&1 || die 'rmdir is required'

LOCK_HELD=0
LOCK_BASE=${TMPDIR:-/tmp}
case "$LOCK_BASE" in
    /*) ;;
    *) die 'TMPDIR must be an absolute path' ;;
esac
reject_path_components "$LOCK_BASE"
LOCK_ROOT="$LOCK_BASE/zerobeat-install-$(id -u)-$(printf '%s' "$PREFIX" | sha256sum | awk '{print $1}').lock"

release_lock() {
    if [ "$LOCK_HELD" -eq 1 ]; then
        rmdir "$LOCK_ROOT" 2>/dev/null || true
        LOCK_HELD=0
    fi
}

acquire_lock() {
    [ ! -L "$LOCK_ROOT" ] || die "refusing symlink lock path: $LOCK_ROOT"
    if [ -e "$LOCK_ROOT" ]; then
        ensure_owned "$LOCK_ROOT" || die "lock path is not owned by the current user: $LOCK_ROOT"
        die "another installer is already operating on PREFIX: $PREFIX"
    fi
    mkdir "$LOCK_ROOT" 2>/dev/null || die "could not acquire installer lock for PREFIX: $PREFIX"
    ensure_owned "$LOCK_ROOT" || {
        rmdir "$LOCK_ROOT" 2>/dev/null || true
        die "installer lock is not owned by the current user: $LOCK_ROOT"
    }
    LOCK_HELD=1
}

acquire_lock
trap release_lock 0
trap 'release_lock; exit 1' 1 2 3 15
recheck_paths

if [ "$ACTION" = uninstall ]; then
    [ -f "$MANIFEST" ] || die 'installer manifest is missing; refusing uninstall'
    verify_installed_pair || die 'installed files are modified, unowned, or do not match the installer manifest'
    recheck_paths
    verify_installed_pair || die 'installed files changed during uninstall preflight; refusing removal'
    rm -f "$BINDIR/zerobeat-cli" "$BINDIR/zerobeatd" "$MANIFEST"
    printf 'Removed ZeroBeat executables and installer manifest; user data was preserved.\n'
    exit 0
fi

command -v curl >/dev/null 2>&1 || die 'curl is required'
command -v tar >/dev/null 2>&1 || die 'tar is required'
command -v mktemp >/dev/null 2>&1 || die 'mktemp is required'
command -v install >/dev/null 2>&1 || die 'install is required'
command -v readelf >/dev/null 2>&1 || die 'readelf is required to validate ELF files'
command -v ldconfig >/dev/null 2>&1 || die 'ldconfig is required to validate shared-library dependencies'
command -v cp >/dev/null 2>&1 || die 'cp is required'

if [ -e "$MANIFEST" ]; then
    verify_installed_pair || die 'existing installation is modified, unowned, or has an invalid installer manifest'
else
    if [ -e "$BINDIR/zerobeat-cli" ] || [ -e "$BINDIR/zerobeatd" ]; then
        die 'existing destination binary has no valid installer manifest; refusing overwrite'
    fi
fi

case "$VERSION" in
    latest)
        BASE_URL=${ZEROBEAT_RELEASE_BASE_URL:-"https://github.com/$REPOSITORY/releases/latest/download"}
        ;;
    v[0-9]*)
        BASE_URL=${ZEROBEAT_RELEASE_BASE_URL:-"https://github.com/$REPOSITORY/releases/download/$VERSION"}
        ;;
    *) die 'VERSION must be latest or a tag beginning with v (for example v0.1.3)' ;;
esac

case "$BASE_URL" in
    https://*) CURL_SECURITY_ARGS='--proto =https --tlsv1.2' ;;
    file://*)
        [ "${ZEROBEAT_ALLOW_LOCAL_TEST:-0}" = 1 ] ||
            die 'file:// URLs require ZEROBEAT_ALLOW_LOCAL_TEST=1'
        CURL_SECURITY_ARGS=''
        ;;
    *) die 'release base must use https://' ;;
esac

read_os_release_value() {
    key=$1
    [ -r /etc/os-release ] || return 1
    value=$(sed -n "s/^${key}=//p" /etc/os-release | sed -n '1p')
    value=${value#\"}
    value=${value%\"}
    [ -n "$value" ] || return 1
    printf '%s' "$value"
}

DISTRO_ID=$(read_os_release_value ID) || die 'could not identify Linux distribution from /etc/os-release'
DISTRO_VERSION=$(read_os_release_value VERSION_ID || true)
case "$DISTRO_ID" in
    arch)
        ARCHIVE_NAME=zerobeat-linux-arch-x86_64.tar.gz
        ;;
    ubuntu)
        [ "$DISTRO_VERSION" = 24.04 ] ||
            die "unsupported Ubuntu version: ${DISTRO_VERSION:-unknown}; prebuilt release requires Ubuntu 24.04"
        ARCHIVE_NAME=zerobeat-linux-x86_64.tar.gz
        ;;
    *)
        die "unsupported Linux distribution: $DISTRO_ID; prebuilt releases support Arch Linux and Ubuntu 24.04"
        ;;
esac

TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/zerobeat-install.XXXXXX")
STAGE_DIR=''
BACKUP_DIR=''
NEW_ONE="$BINDIR/.zerobeat-cli.new.$$"
NEW_DAEMON="$BINDIR/.zerobeatd.new.$$"
NEW_MANIFEST="$MANIFEST_DIR/.install-manifest.new.$$"
HAD_ONE=0
HAD_DAEMON=0
HAD_MANIFEST=0
TRANSACTION_ACTIVE=0

cleanup() {
    [ -z "$STAGE_DIR" ] || [ ! -d "$STAGE_DIR" ] || rm -rf "$STAGE_DIR"
    [ -z "$BACKUP_DIR" ] || [ ! -d "$BACKUP_DIR" ] || rm -rf "$BACKUP_DIR"
    [ ! -d "$TMP_DIR" ] || rm -rf "$TMP_DIR"
    release_lock
}

rollback() {
    status=$?
    set +e
    trap - 0 1 2 3 15
    rollback_failed=0
    if [ "$TRANSACTION_ACTIVE" -eq 1 ]; then
        rm -f "$NEW_ONE" "$NEW_DAEMON" "$NEW_MANIFEST" || rollback_failed=1
        if [ "$HAD_ONE" -eq 1 ]; then
            cp -p "$BACKUP_DIR/zerobeat-cli" "$BINDIR/zerobeat-cli" || rollback_failed=1
        else
            rm -f "$BINDIR/zerobeat-cli" || rollback_failed=1
        fi
        if [ "$HAD_DAEMON" -eq 1 ]; then
            cp -p "$BACKUP_DIR/zerobeatd" "$BINDIR/zerobeatd" || rollback_failed=1
        else
            rm -f "$BINDIR/zerobeatd" || rollback_failed=1
        fi
        if [ "$HAD_MANIFEST" -eq 1 ]; then
            cp -p "$BACKUP_DIR/install-manifest" "$MANIFEST" || rollback_failed=1
        else
            rm -f "$MANIFEST" || rollback_failed=1
        fi
    fi
    cleanup || rollback_failed=1
    if [ "$rollback_failed" -ne 0 ]; then
        printf 'install.sh: rollback encountered an error; original status %s preserved\n' "$status" >&2
    fi
    exit "$status"
}
trap rollback 0 1 2 3 15

download() {
    url=$1
    destination=$2
    if [ -n "$CURL_SECURITY_ARGS" ]; then
        # shellcheck disable=SC2086
        curl $CURL_SECURITY_ARGS -fsSL --retry 3 --connect-timeout 15 -o "$destination" -- "$url"
    else
        curl -fsSL -o "$destination" -- "$url"
    fi
}

validate_elf() {
    path=$1
    label=$2
    header="$TMP_DIR/$label.header"
    program="$TMP_DIR/$label.program"
    dynamic="$TMP_DIR/$label.dynamic"

    readelf -h "$path" > "$header" 2>&1 || die "readelf could not inspect $label"
    grep -E 'Class:.*ELF64' "$header" >/dev/null 2>&1 ||
        die "$label is not an ELF64 binary"
    grep -E 'Machine:.*(Advanced Micro Devices X86-64|X86-64)' "$header" >/dev/null 2>&1 ||
        die "$label is not an x86-64 binary"

    readelf -l "$path" > "$program" 2>&1 || die "readelf could not inspect $label program headers"
    interpreter=$(sed -n 's/.*Requesting program interpreter: \(.*\)]/\1/p' "$program")
    [ -n "$interpreter" ] || die "$label has no program interpreter"
    [ -f "$interpreter" ] || die "$label requests missing program interpreter: $interpreter"

    readelf -d "$path" > "$dynamic" 2>&1 || die "readelf could not inspect $label dynamic section"
    if grep -E 'RPATH|RUNPATH' "$dynamic" >/dev/null 2>&1; then
        die "$label contains an unsafe RPATH or RUNPATH"
    fi
    needed=$(sed -n 's/.*Shared library: \[\(.*\)\].*/\1/p' "$dynamic")
    [ -n "$needed" ] || die "$label has no dynamic dependencies"
    while IFS= read -r soname; do
        [ -n "$soname" ] || continue
        if ! ldconfig -p | awk -v name="$soname" '$1 == name && $0 ~ /x86-64/ { found=1 } END { exit(found ? 0 : 1) }'; then
            die "$label has an unavailable shared-library dependency: $soname"
        fi
    done <<EOF
$needed
EOF
}

CHECKSUM_NAME="$ARCHIVE_NAME.sha256"
ARCHIVE_PATH="$TMP_DIR/$ARCHIVE_NAME"
CHECKSUM_PATH="$TMP_DIR/$CHECKSUM_NAME"
download "$BASE_URL/$ARCHIVE_NAME" "$ARCHIVE_PATH"
download "$BASE_URL/$CHECKSUM_NAME" "$CHECKSUM_PATH"

EXPECTED=$(awk 'NF >= 1 { print $1; exit }' "$CHECKSUM_PATH")
valid_digest "$EXPECTED" || die 'checksum file does not contain a SHA-256 digest'
ACTUAL=$(sha256sum "$ARCHIVE_PATH" | awk '{print $1}')
[ "$ACTUAL" = "$EXPECTED" ] || die 'archive SHA-256 verification failed'

MANIFEST_LIST="$TMP_DIR/archive-manifest"
tar -tzf "$ARCHIVE_PATH" > "$MANIFEST_LIST" || die 'archive is not a readable gzip tar archive'
for entry in zerobeat-cli zerobeatd README.md LICENSE; do
    grep -F -x "$entry" "$MANIFEST_LIST" >/dev/null 2>&1 ||
        die "archive is missing $entry"
done
while IFS= read -r entry; do
    case "$entry" in
        zerobeat-cli|zerobeatd|README.md|LICENSE) ;;
        *) die "archive contains unexpected path: $entry" ;;
    esac
done < "$MANIFEST_LIST"

STAGE_DIR=$(mktemp -d "$TMP_DIR/stage.XXXXXX")
tar -xzf "$ARCHIVE_PATH" -C "$STAGE_DIR" --no-same-owner --no-same-permissions
for binary in zerobeat-cli zerobeatd; do
    path="$STAGE_DIR/$binary"
    [ -f "$path" ] || die "$binary is not a regular file"
    [ ! -L "$path" ] || die "$binary is a symlink"
    [ -x "$path" ] || die "$binary is not executable"
    validate_elf "$path" "$binary"
done

if [ -e "$PREFIX" ] && [ ! -d "$PREFIX" ]; then
    die "PREFIX is not a directory: $PREFIX"
fi
recheck_paths
mkdir -p "$BINDIR" "$MANIFEST_DIR"
recheck_paths
BACKUP_DIR=$(mktemp -d "$TMP_DIR/backup.XXXXXX")
if [ -e "$BINDIR/zerobeat-cli" ]; then
    cp -p "$BINDIR/zerobeat-cli" "$BACKUP_DIR/zerobeat-cli"
    HAD_ONE=1
fi
if [ -e "$BINDIR/zerobeatd" ]; then
    cp -p "$BINDIR/zerobeatd" "$BACKUP_DIR/zerobeatd"
    HAD_DAEMON=1
fi
if [ -e "$MANIFEST" ]; then
    cp -p "$MANIFEST" "$BACKUP_DIR/install-manifest"
    HAD_MANIFEST=1
fi
TRANSACTION_ACTIVE=1

recheck_paths
install -m 755 "$STAGE_DIR/zerobeat-cli" "$NEW_ONE"
install -m 755 "$STAGE_DIR/zerobeatd" "$NEW_DAEMON"
ONE_HASH=$(sha256sum "$STAGE_DIR/zerobeat-cli" | awk '{print $1}')
DAEMON_HASH=$(sha256sum "$STAGE_DIR/zerobeatd" | awk '{print $1}')
printf 'zerobeat-cli %s\nzerobeatd %s\n' "$ONE_HASH" "$DAEMON_HASH" > "$NEW_MANIFEST"
chmod 644 "$NEW_MANIFEST"
recheck_paths
mv -f "$NEW_ONE" "$BINDIR/zerobeat-cli"
mv -f "$NEW_DAEMON" "$BINDIR/zerobeatd"
mv -f "$NEW_MANIFEST" "$MANIFEST"
TRANSACTION_ACTIVE=0
trap - 0 1 2 3 15
cleanup

printf 'Installed ZeroBeat (%s) to %s\n' "$VERSION" "$BINDIR"
case ":${PATH:-}:" in
    *":$BINDIR:"*) ;;
    *) printf 'Add this directory to PATH: export PATH="%s:%s"\n' "$BINDIR" "\$PATH" ;;
esac
printf 'Run: zerobeat-cli\n'
