# This launcher only prepares the native executable and forwards its arguments.
# Keep all bootstrap messages on stderr so stdout contains only CLI results.
Import-Module Microsoft.PowerShell.Utility
Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false)
[Console]::InputEncoding = $utf8
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8

function Get-BulbulBinary {
    $skillDir = Split-Path -Parent $PSScriptRoot
    $manifest = Get-Content -LiteralPath (Join-Path $skillDir 'assets/runtime.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $version = [string]$manifest.version
    $repository = [string]$manifest.repository
    if ($version -notmatch '^\d+\.\d+\.\d+([-+][0-9A-Za-z.+-]+)?$' -or $repository -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') {
        throw 'Invalid Bulbul distribution metadata.'
    }
    $arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    if ($arch -ne 'AMD64') { throw 'This package currently supports Windows x64.' }
    $target = 'x86_64-pc-windows-msvc'
    $localBinary = Join-Path $skillDir 'bin/bulbul.exe'
    if (Test-Path -LiteralPath $localBinary -PathType Leaf) {
        $localVersion = & $localBinary --version 2>$null
        if ($LASTEXITCODE -eq 0 -and $localVersion -eq "bulbul $version") { return $localBinary }
    }

    $asset = "bulbul-$version-$target.exe"
    $cacheDir = Join-Path $skillDir "bin/$version/$target"
    $binary = Join-Path $cacheDir 'bulbul.exe'
    $hashPattern = '^([0-9a-fA-F]{64})  ' + [regex]::Escape($asset) + '\r?\n?$'
    if ((Test-Path -LiteralPath $binary -PathType Leaf) -and (Test-Path -LiteralPath "$binary.sha256" -PathType Leaf)) {
        $checksum = Get-Content -LiteralPath "$binary.sha256" -Raw
        if ($checksum -match $hashPattern) {
            $expected = $Matches[1]
            if ((Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash -eq $expected) {
                $cachedVersion = & $binary --version 2>$null
                if ($LASTEXITCODE -ne 0 -or $cachedVersion -ne "bulbul $version") { throw 'Cached executable has an unexpected version.' }
                return $binary
            }
        }
    }

    [void][IO.Directory]::CreateDirectory($cacheDir)
    $partial = Join-Path $cacheDir ('.download.' + [guid]::NewGuid().ToString('N') + '.exe')
    $checksumPath = "$partial.sha256"
    $base = "https://github.com/$repository/releases/download/v$version"
    [Console]::Error.WriteLine("Bulbul: preparing version $version for $target...")
    try {
        try {
            Invoke-WebRequest -Uri "$base/$asset.sha256" -OutFile $checksumPath -UseBasicParsing -TimeoutSec 120 | Out-Null
        } catch {
            throw "Version $version is not available yet. See https://github.com/$repository/releases."
        }
        $checksum = Get-Content -LiteralPath $checksumPath -Raw
        if ($checksum -notmatch $hashPattern) { throw 'Invalid executable checksum.' }
        $expected = $Matches[1]
        try {
            Invoke-WebRequest -Uri "$base/$asset" -OutFile $partial -UseBasicParsing -TimeoutSec 120 | Out-Null
        } catch {
            throw 'Executable download failed. Try again later.'
        }
        if ((Get-FileHash -LiteralPath $partial -Algorithm SHA256).Hash -ne $expected) { throw 'Executable checksum verification failed.' }
        $downloadedVersion = & $partial --version 2>$null
        if ($LASTEXITCODE -ne 0 -or $downloadedVersion -ne "bulbul $version") { throw 'Downloaded executable has an unexpected version.' }
        try {
            Move-Item -LiteralPath $partial -Destination $binary -Force
        } catch {
            # Another launcher may have completed the same verified download.
            if (-not (Test-Path -LiteralPath $binary -PathType Leaf) -or (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash -ne $expected) { throw }
        }
        Move-Item -LiteralPath $checksumPath -Destination "$binary.sha256" -Force
        return $binary
    } finally {
        foreach ($path in @($partial, $checksumPath)) {
            if (Test-Path -LiteralPath $path -PathType Leaf) { Remove-Item -LiteralPath $path -Force }
        }
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    try { $binary = Get-BulbulBinary } catch {
        [Console]::Error.WriteLine('Bulbul: ' + $_.Exception.Message)
        exit 1
    }
    & $binary @args
    exit $LASTEXITCODE
}
