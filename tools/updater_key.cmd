<# :
@echo off
rem The key that signs Game Repo's updates (docs/PLAN.md, "In-app updates", D135), made once
rem on your PC: double-click this file. It runs the PowerShell below with Windows' own tools
rem and the Game Repo you have installed, so nothing needs installing. The batch lines are a
rem comment to PowerShell.
setlocal
set "GAMEREPO_KEY_SELF=%~f0"
powershell -NoProfile -ExecutionPolicy Bypass -Command "iex ((Get-Content -LiteralPath $env:GAMEREPO_KEY_SELF) -join [char]10)"
set "GAMEREPO_KEY_CODE=%errorlevel%"
if not defined GAMEREPO_KEY_QUIET pause
exit /b %GAMEREPO_KEY_CODE%
#>

# Game Repo, Game Repo Server and Game Repo Go install an update only when it's signed with
# this key: each has its public half built in, and CI signs each release's installers with
# the private half. CI keeps the key as three repository secrets, which only the
# repository's owner can add. This makes the key (minisign's Ed25519, as Tauri's updater
# reads it, with a random password) with the Game Repo installed on this PC (0.5.41 or
# later: `game-repo.exe --make-update-key`), keeps it in Documents\Game Repo update key\,
# and adds the three secrets: with GitHub CLI if it's installed and signed in, otherwise by
# opening GitHub's page for each secret with its value copied, ready to paste. Run again,
# it uses the key it made before; it never makes a second one.
#
# For the test (tests/updater_key.py): GAMEREPO_KEY_DIR makes the key there instead,
# GAMEREPO_UPDATE_KEY_PROGRAM names the game-repo.exe to make it with, and GAMEREPO_KEY_QUIET
# only makes it (no secrets, no questions, no pause).

$ErrorActionPreference = 'Stop'
$repo = 'DrFlGd/Game-Repo'
$first = [version]'0.5.41'
$names = 'UPDATER_PRIVATE_KEY', 'UPDATER_KEY_PASSWORD', 'UPDATER_PUBKEY'
$dir = $env:GAMEREPO_KEY_DIR
if (-not $dir) { $dir = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'Game Repo update key' }
$quiet = [bool]$env:GAMEREPO_KEY_QUIET
$privateFile = Join-Path $dir 'private.key'
$publicFile = Join-Path $dir 'public.key'
$passFile = Join-Path $dir 'password.txt'

function Say([string]$text) { Write-Host $text }

# The Game Repo installed on this PC: where its installer puts it, or where Windows says it is
function Find-GameRepo {
    if ($env:GAMEREPO_UPDATE_KEY_PROGRAM) { return $env:GAMEREPO_UPDATE_KEY_PROGRAM }
    $usual = Join-Path $env:LOCALAPPDATA 'Game Repo\game-repo.exe'
    if (Test-Path -LiteralPath $usual) { return $usual }
    foreach ($root in 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall') {
        $entry = Get-ChildItem $root -ErrorAction SilentlyContinue | Get-ItemProperty -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -eq 'Game Repo' } | Select-Object -First 1
        if ($entry -and $entry.InstallLocation) {
            $exe = Join-Path ($entry.InstallLocation.Trim('"')) 'game-repo.exe'
            if (Test-Path -LiteralPath $exe) { return $exe }
        }
    }
    throw "Game Repo isn't installed on this PC. Install Game Repo $first or later from https://github.com/$repo/releases first."
}

# The key, made by Game Repo in $dir with $password
function New-Key([string]$password) {
    $exe = Find-GameRepo
    $version = $null
    [void][version]::TryParse(((Get-Item -LiteralPath $exe).VersionInfo.ProductVersion -replace '[^0-9.].*$', ''), [ref]$version)
    if (-not $version -or $version -lt $first) { throw "The Game Repo installed ($exe, version $version) is older than $first, which makes the key. Install the newest from https://github.com/$repo/releases first." }
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $errors = Join-Path ([IO.Path]::GetTempPath()) ('gamerepo-key-' + [guid]::NewGuid() + '.txt')
    $env:GAMEREPO_UPDATE_KEY_PASSWORD = $password
    try {
        $p = Start-Process -FilePath $exe -ArgumentList @('--make-update-key', ('"' + $dir + '"')) -Wait -PassThru -NoNewWindow -RedirectStandardError $errors
        $said = if (Test-Path -LiteralPath $errors) { (Get-Content -LiteralPath $errors -Raw) } else { '' }
        if ($p.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $privateFile) -or -not (Test-Path -LiteralPath $publicFile)) {
            throw "Game Repo couldn't make the key (exit $($p.ExitCode)): $said"
        }
    } finally {
        Remove-Item Env:\GAMEREPO_UPDATE_KEY_PASSWORD -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $errors -ErrorAction SilentlyContinue
    }
}

try {
    Say ''
    Say "Game Repo: the key that signs its updates"
    Say '========================================='
    if (Test-Path -LiteralPath $privateFile) {
        if (-not (Test-Path -LiteralPath $passFile) -or -not (Test-Path -LiteralPath $publicFile)) { throw "$privateFile is there but public.key or password.txt beside it isn't. Put them back, or move the folder away to make a new key (programs built with the old key can't update to ones signed with a new one)." }
        $password = (Get-Content -LiteralPath $passFile -TotalCount 1).Trim()
        Say "Using the key made before: $dir"
    } else {
        # a password of 32 letters and digits
        $alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789'
        $bytes = New-Object byte[] 32
        $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
        $rng.GetBytes($bytes)
        $password = -join ($bytes | ForEach-Object { $alphabet[$_ % $alphabet.Length] })
        New-Key $password
        [IO.File]::WriteAllText($passFile, $password + "`r`n", [Text.Encoding]::ASCII)
        [IO.File]::WriteAllText((Join-Path $dir 'README.txt'), @"
This is the key that signs Game Repo's updates, made $(Get-Date -Format 'yyyy-MM-dd').
Game Repo, Game Repo Server and Game Repo Go install an update only when it's
signed with it.

Keep this folder (private.key, password.txt and public.key) somewhere safe, with a
backup copy elsewhere. Nobody else should have it: anyone with private.key and its
password can sign an update your computers and devices would install.

GitHub keeps a copy as the repository secrets UPDATER_PRIVATE_KEY, UPDATER_KEY_PASSWORD
and UPDATER_PUBKEY, which CI signs each release with. To add them again, run
updater_key.cmd again (it's in the repository's tools folder): it uses this key, not a
new one.

If the key is ever lost, a new one can be made, but the copies installed until then
won't update to a release signed with it: install that release by hand once on each
computer and device, and they update themselves again from then on.
"@, [Text.Encoding]::ASCII)
        Say "Made a new key: $dir"
        Say 'Keep that folder safe, with a backup copy (README.txt in it says why).'
    }
    $publicText = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String((Get-Content -LiteralPath $publicFile -Raw).Trim()))
    if ($publicText -match 'public key: ([0-9A-F]{16})') { Say "Its key ID: $($Matches[1])" }
    if ($quiet) { exit 0 }

    $values = @{
        'UPDATER_PRIVATE_KEY' = (Get-Content -LiteralPath $privateFile -Raw).Trim()
        'UPDATER_KEY_PASSWORD' = $password
        'UPDATER_PUBKEY' = (Get-Content -LiteralPath $publicFile -Raw).Trim()
    }
    Say ''
    $gh = Get-Command gh -ErrorAction SilentlyContinue
    $signedIn = $false
    if ($gh) {
        # Windows PowerShell turns a program's redirected messages into errors
        $ErrorActionPreference = 'Continue'
        & gh auth status --hostname github.com *> $null
        $signedIn = ($LASTEXITCODE -eq 0)
        $ErrorActionPreference = 'Stop'
    }
    if ($signedIn) {
        Say "GitHub CLI is signed in, so it can add the three secrets to $repo now."
        $answer = Read-Host 'Add them? [Y/n]'
        if ($answer -notmatch '^[nN]') {
            foreach ($name in $names) {
                & gh secret set $name --repo $repo --body $values[$name]
                if ($LASTEXITCODE -ne 0) { throw "GitHub CLI couldn't add $name." }
            }
            Say 'All three secrets are added. The next release is signed with this key.'
            exit 0
        }
    }
    Say "Now the three secrets, one at a time, on GitHub's page for a new repository secret."
    Say 'You need to be signed in to GitHub as the repository owner.'
    $i = 0
    foreach ($name in $names) {
        $i++
        Set-Clipboard -Value $values[$name]
        Start-Process "https://github.com/$repo/settings/secrets/actions/new"
        Say ''
        Say "Secret $i of 3, in the page that just opened:"
        Say "  Name:   $name"
        Say '  Secret: press Ctrl+V (it is copied already)'
        Say '  then press Add secret.'
        Read-Host 'Press Enter here once it is added' | Out-Null
    }
    # the password and the key off the clipboard
    Set-Clipboard -Value ' '
    Say ''
    Say "Done. All three secrets should now be listed at https://github.com/$repo/settings/secrets/actions"
    Say 'The next release is signed with this key.'
    exit 0
} catch {
    Say ''
    Say "That didn't work: $($_.Exception.Message)"
    exit 1
}
