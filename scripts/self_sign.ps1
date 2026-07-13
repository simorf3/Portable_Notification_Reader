<#
.SYNOPSIS
    Create a FREE self-signed code-signing certificate for Portable Notification
    Reader, optionally sign a local exe with it, and print the base64 string you
    paste into the GitHub Actions secret so CI signs every release for you.

.DESCRIPTION
    A self-signed certificate does NOT remove the first SmartScreen warning
    (only a paid CA certificate does). What it DOES give you:
      * a stable "publisher" identity stamped into the exe, and
      * the ability for a user to install that certificate into "Trusted
        Publishers" ONCE and then run every future signed version with no prompt.

    Run this on a Windows machine (PowerShell). It never uploads anything.

.PARAMETER Publisher
    The name shown as the publisher. Defaults to "Portable Notification Reader".

.PARAMETER Password
    Password used to protect the exported .pfx. If omitted you'll be prompted.

.PARAMETER SignExe
    Optional path to an exe to sign right away (for local testing).

.EXAMPLE
    # Generate a cert, export cert.pfx, print the base64 secret:
    .\self_sign.ps1 -Password "s3cret!"

.EXAMPLE
    # Also sign a locally built exe:
    .\self_sign.ps1 -Password "s3cret!" -SignExe .\PortableNotificationReader.exe
#>

[CmdletBinding()]
param(
    [string]$Publisher = "Portable Notification Reader",
    [string]$Password,
    [string]$SignExe
)

$ErrorActionPreference = "Stop"

if (-not $Password) {
    $secure = Read-Host "Choose a password for the .pfx" -AsSecureString
} else {
    $secure = ConvertTo-SecureString -String $Password -Force -AsPlainText
}

Write-Host "==> Creating self-signed code-signing certificate for '$Publisher'..."
$cert = New-SelfSignedCertificate `
    -Type CodeSigningCert `
    -Subject "CN=$Publisher" `
    -KeyUsage DigitalSignature `
    -FriendlyName "$Publisher (self-signed)" `
    -CertStoreLocation "Cert:\CurrentUser\My" `
    -NotAfter (Get-Date).AddYears(5)

$pfxPath = Join-Path (Get-Location) "cert.pfx"
Export-PfxCertificate -Cert $cert -FilePath $pfxPath -Password $secure | Out-Null
Write-Host "==> Exported certificate to $pfxPath"

# Locate the newest signtool.exe from the Windows SDK.
$signtool = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName

if ($SignExe) {
    if (-not $signtool) {
        Write-Warning "signtool.exe not found (install the Windows SDK). Skipping local signing."
    } elseif (-not (Test-Path $SignExe)) {
        Write-Warning "Exe not found: $SignExe. Skipping local signing."
    } else {
        $plain = [Runtime.InteropServices.Marshal]::PtrToStringAuto(
                    [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure))
        Write-Host "==> Signing $SignExe ..."
        & $signtool sign /f $pfxPath /p $plain /fd sha256 `
            /tr http://timestamp.digicert.com /td sha256 $SignExe
        & $signtool verify /pa $SignExe
    }
}

# Print the base64 blob for the GitHub Actions secret.
$b64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($pfxPath))
$b64Path = Join-Path (Get-Location) "cert.pfx.base64.txt"
$b64 | Out-File -Encoding ascii $b64Path

Write-Host ""
Write-Host "======================================================================"
Write-Host "Next steps to make CI sign every release automatically:"
Write-Host ""
Write-Host "  1. Open your repo -> Settings -> Secrets and variables -> Actions."
Write-Host "  2. New repository secret:"
Write-Host "        Name : CODESIGN_PFX_BASE64"
Write-Host "        Value: (contents of $b64Path)"
Write-Host "  3. New repository secret:"
Write-Host "        Name : CODESIGN_PASSWORD"
Write-Host "        Value: the password you just chose"
Write-Host ""
Write-Host "The build.yml workflow signs the exe only when these secrets exist,"
Write-Host "so nothing changes until you add them."
Write-Host ""
Write-Host "SECURITY: keep cert.pfx and $b64Path private. Do NOT commit them."
Write-Host "======================================================================"
