# Builds the Dos60S firmware inside the "Docker" WSL distro (QMK lives in its
# Linux filesystem, ~/qmk_firmware, for full build speed) and copies the .hex
# back to firmware\.
# Usage: .\build_firmware.ps1 [keymap]    (default: default)
param([string]$Keymap = 'default')
$ErrorActionPreference = 'Stop'

# Pinned build image (update deliberately: docker pull ghcr.io/qmk/qmk_cli,
# then copy the new digest from `docker image inspect`).
$image = 'ghcr.io/qmk/qmk_cli@sha256:b7d7fa8fb4432b569931de5ad59098cb788f440ed61a62c5126746b71aee0f4a'

# Everything below is pasted into a bash command line, so only allow safe text.
if ($Keymap -notmatch '^[A-Za-z0-9_-]+$') { throw "Invalid keymap name '$Keymap'." }
if ($PSScriptRoot -notmatch '^[A-Za-z0-9 _.:\\-]+$') {
    throw "Project path has characters a shell would interpret; move it to a plain path: $PSScriptRoot"
}

# --exec runs wslpath directly, with no shell in between. The bash step below
# needs a shell, which is why the path is allow-listed above.
$root = $PSScriptRoot.Replace('\', '/')
$src = wsl -d Docker --exec wslpath -a "$root/firmware/Dos60S/QMK/dosboards"
$mod = wsl -d Docker --exec wslpath -a "$root/qmk_module"
$out = wsl -d Docker --exec wslpath -a "$root/firmware"
$hex = "dosboards_dos60s_$Keymap.hex"

$steps = @(
    "cp -rT '$src' ~/qmk_firmware/keyboards/dosboards"
    # The which_layer community module, as "brandondoster/which_layer".
    "mkdir -p ~/qmk_firmware/modules/brandondoster && cp -rT '$mod' ~/qmk_firmware/modules/brandondoster"
    "docker run --rm --user `$(id -u):`$(id -g) -v ~/qmk_firmware:/qmk_firmware -w /qmk_firmware -e MAKEFLAGS=-j`$(nproc) $image make dosboards/dos60s:$Keymap"
    "cp ~/qmk_firmware/$hex '$out/'"
)
wsl -d Docker -- bash -lc ($steps -join ' && ')
if ($LASTEXITCODE -ne 0) { throw "Firmware build failed." }
Write-Host "Built firmware\$hex"
