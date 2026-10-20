<# :
@echo off
rem Game Repo Go's Android signing key (docs/PLAN.md, "Phase 9b design", D104), made once on
rem your PC: double-click this file. It runs the PowerShell below with Windows' own tools,
rem so nothing needs installing. The batch lines are a comment to PowerShell.
setlocal
set "GAMEREPO_KEY_SELF=%~f0"
powershell -NoProfile -ExecutionPolicy Bypass -Command "iex ((Get-Content -LiteralPath $env:GAMEREPO_KEY_SELF) -join [char]10)"
set "GAMEREPO_KEY_CODE=%errorlevel%"
if not defined GAMEREPO_KEY_QUIET pause
exit /b %GAMEREPO_KEY_CODE%
#>

# An APK has to be signed with the same key every time, or Android refuses each update
# over the last one. CI keeps the key as two repository secrets, which only the repository's
# owner can add. This makes the key (an RSA 2048 key and a certificate good for 50 years,
# in a PKCS #12 file with a random password), keeps it in Documents\Game Repo Go signing
# key\, and adds the two secrets: with GitHub CLI if it's installed and signed in,
# otherwise by opening GitHub's page for each secret with its value copied, ready to paste.
# Run again, it uses the key it made before; it never makes a second one.
#
# For the test (tests/android_key.py): GAMEREPO_KEY_DIR makes the key there instead,
# GAMEREPO_KEY_QUIET only makes it (no secrets, no questions, no pause), and
# GAMEREPO_KEY_METHOD=store makes it the second way below.

$ErrorActionPreference = 'Stop'
$repo = 'DrFlGd/Game-Repo'
$names = 'ANDROID_KEYSTORE_BASE64', 'ANDROID_KEYSTORE_PASSWORD'
$dir = $env:GAMEREPO_KEY_DIR
if (-not $dir) { $dir = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'Game Repo Go signing key' }
$quiet = [bool]$env:GAMEREPO_KEY_QUIET
$keyFile = Join-Path $dir 'game-repo-go.p12'
$passFile = Join-Path $dir 'password.txt'

function Say([string]$text) { Write-Host $text }

# The certificate in the file, read without keeping its key anywhere
function Read-Key([string]$file, [string]$password) {
    New-Object Security.Cryptography.X509Certificates.X509Certificate2 -ArgumentList $file, $password, ([Security.Cryptography.X509Certificates.X509KeyStorageFlags]::EphemeralKeySet)
}

# .NET's CertificateRequest (Windows 10 and 11 have it); failing that, Windows' own
# New-SelfSignedCertificate, made in your certificate store, exported and taken out again
function New-Key([string]$file, [string]$password) {
    $subject = 'CN=Game Repo Go, O=DrFlGd'
    if ($env:GAMEREPO_KEY_METHOD -ne 'store') {
        try {
            $rsa = [Security.Cryptography.RSA]::Create(2048)
            $request = New-Object Security.Cryptography.X509Certificates.CertificateRequest -ArgumentList $subject, $rsa, ([Security.Cryptography.HashAlgorithmName]::SHA256), ([Security.Cryptography.RSASignaturePadding]::Pkcs1)
            $made = $request.CreateSelfSigned([DateTimeOffset]::UtcNow.AddDays(-1), [DateTimeOffset]::UtcNow.AddYears(50))
            [IO.File]::WriteAllBytes($file, $made.Export([Security.Cryptography.X509Certificates.X509ContentType]::Pfx, $password))
            return
        } catch {
            Say "(.NET couldn't make it: $($_.Exception.Message); trying Windows' own way)"
        }
    }
    $made = New-SelfSignedCertificate -Subject $subject -FriendlyName 'Game Repo Go (Android signing key)' -KeyAlgorithm RSA -KeyLength 2048 -HashAlgorithm SHA256 -KeyExportPolicy Exportable -KeyUsage DigitalSignature -NotBefore (Get-Date).AddDays(-1) -NotAfter (Get-Date).AddYears(50) -CertStoreLocation 'Cert:\CurrentUser\My'
    try {
        Export-PfxCertificate -Cert $made -FilePath $file -Password (ConvertTo-SecureString $password -AsPlainText -Force) | Out-Null
    } finally {
        Remove-Item -LiteralPath ('Cert:\CurrentUser\My\' + $made.Thumbprint) -DeleteKey
    }
}

try {
    Say ''
    Say 'Game Repo Go: the key that signs its Android app'
    Say '================================================'
    if (Test-Path -LiteralPath $keyFile) {
        if (-not (Test-Path -LiteralPath $passFile)) { throw "$keyFile is there but its password.txt isn't. Put password.txt back beside it, or move the folder away to make a new key (an APK signed with a new key can't update one signed with the old)." }
        $password = (Get-Content -LiteralPath $passFile -TotalCount 1).Trim()
        $cert = Read-Key $keyFile $password
        Say "Using the key made before: $keyFile"
    } else {
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        # a password of 32 letters and digits
        $alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789'
        $bytes = New-Object byte[] 32
        $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
        $rng.GetBytes($bytes)
        $password = -join ($bytes | ForEach-Object { $alphabet[$_ % $alphabet.Length] })
        New-Key $keyFile $password
        [IO.File]::WriteAllText($passFile, $password + "`r`n", [Text.Encoding]::ASCII)
        [IO.File]::WriteAllText((Join-Path $dir 'README.txt'), @"
This is the key that signs Game Repo Go for Android, made $(Get-Date -Format 'yyyy-MM-dd').

Keep this folder (game-repo-go.p12 and password.txt) somewhere safe, with a backup
copy elsewhere. Nobody else should have it: anyone with it can sign an app that
installs over yours.

GitHub keeps a copy as the repository secrets ANDROID_KEYSTORE_BASE64 and
ANDROID_KEYSTORE_PASSWORD, which CI signs each release's APK with. To add them again,
run android_signing_key.cmd again (it's in the repository's tools folder): it uses this
key, not a new one.

If the key is ever lost, the next Game Repo Go can't install over the one on your
devices: uninstall it there first and pair it with the library again. The games
it added stay in ES-DE's folders.
"@, [Text.Encoding]::ASCII)
        $cert = Read-Key $keyFile $password
        Say "Made a new key: $keyFile"
        Say 'Keep that folder safe, with a backup copy (README.txt in it says why).'
    }
    $sha = [Security.Cryptography.SHA256]::Create()
    $fingerprint = ($sha.ComputeHash($cert.RawData) | ForEach-Object { $_.ToString('X2') }) -join ':'
    Say "Its certificate's SHA-256: $fingerprint"
    Say '(each release says which certificate signed its APK, so you can check it is this one)'
    if ($quiet) { exit 0 }

    $values = @{ 'ANDROID_KEYSTORE_BASE64' = [Convert]::ToBase64String([IO.File]::ReadAllBytes($keyFile)); 'ANDROID_KEYSTORE_PASSWORD' = $password }
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
        Say "GitHub CLI is signed in, so it can add the two secrets to $repo now."
        $answer = Read-Host 'Add them? [Y/n]'
        if ($answer -notmatch '^[nN]') {
            foreach ($name in $names) {
                & gh secret set $name --repo $repo --body $values[$name]
                if ($LASTEXITCODE -ne 0) { throw "GitHub CLI couldn't add $name." }
            }
            Say 'Both secrets are added. The next release signs its APK with this key.'
            exit 0
        }
    }
    Say "Now the two secrets, one at a time, on GitHub's page for a new repository secret."
    Say 'You need to be signed in to GitHub as the repository owner.'
    $i = 0
    foreach ($name in $names) {
        $i++
        Set-Clipboard -Value $values[$name]
        Start-Process "https://github.com/$repo/settings/secrets/actions/new"
        Say ''
        Say "Secret $i of 2, in the page that just opened:"
        Say "  Name:   $name"
        Say '  Secret: press Ctrl+V (it is copied already)'
        Say '  then press Add secret.'
        Read-Host 'Press Enter here once it is added' | Out-Null
    }
    # the password and the key off the clipboard
    Set-Clipboard -Value ' '
    Say ''
    Say "Done. Both secrets should now be listed at https://github.com/$repo/settings/secrets/actions"
    Say 'The next release signs its APK with this key.'
    exit 0
} catch {
    Say ''
    Say "That didn't work: $($_.Exception.Message)"
    exit 1
}
