# Schema discovery is local; other commands prepare and run the native executable.
# Keep all bootstrap messages on stderr so stdout contains only CLI results.
Import-Module Microsoft.PowerShell.Utility
Set-StrictMode -Version 3.0
$ErrorActionPreference = 'Stop'
$utf8 = [Text.UTF8Encoding]::new($false)
[Console]::InputEncoding = $utf8
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8

function Get-BulbulCacheRoot {
    if ($env:BULBUL_CACHE_DIR) { return [IO.Path]::GetFullPath($env:BULBUL_CACHE_DIR) }
    $directory = (Get-Location).Path
    while ($directory) {
        if (Test-Path -LiteralPath (Join-Path $directory '.git')) { return Join-Path $directory '.tmp/bulbul' }
        $parent = Split-Path -Parent $directory
        if ($parent -eq $directory) { break }
        $directory = $parent
    }
    if (-not $env:LOCALAPPDATA) { throw '无法找到缓存目录。请在可写的项目目录运行，或设置 BULBUL_CACHE_DIR。' }
    return Join-Path $env:LOCALAPPDATA 'bulbul-birding/cache'
}

function Show-BulbulTools {
    param([string[]]$Names)
    if ($Names -contains '--help' -or $Names -contains '-h') {
        [Console]::Out.WriteLine('Usage: bulbul list [TOOL ...]'); return 0
    }
    if (@($Names | Where-Object { $_ -notmatch '^[a-z][a-z0-9_]*$' }).Count -gt 0) {
        [Console]::Error.WriteLine('{"error":"命令参数无效，请运行 bulbul list --help 查看用法。","code":"invalid_arguments"}'); return 2
    }
    $skillDir = Split-Path -Parent $PSScriptRoot
    $tools = @(Get-Content -LiteralPath (Join-Path $skillDir 'assets/tools.jsonl') -Encoding UTF8 | ForEach-Object { ConvertFrom-Json -InputObject $_ })
    $selected = @($tools)
    if ($Names.Count -gt 0) {
        $selected = @()
        foreach ($name in $Names) {
            $tool = @($tools | Where-Object { $_.name -eq $name })
            if ($tool.Count -ne 1) {
                [Console]::Error.WriteLine('{"error":"没有这个 Bulbul 工具，请运行 list 查看可用工具。","code":"operation_failed"}'); return 1
            }
            $selected += $tool[0]
        }
    }
    [Console]::Out.WriteLine((ConvertTo-Json -InputObject @($selected) -Depth 100 -Compress))
    return 0
}

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
    $cacheDir = Join-Path (Get-BulbulCacheRoot) "$version/$target"
    $binary = Join-Path $cacheDir 'bulbul.exe'
    $hashPattern = '^([0-9a-fA-F]{64})  ' + [regex]::Escape($asset) + '\r?\n?$'
    foreach ($candidate in @($binary, (Join-Path $skillDir "bin/$version/$target/bulbul.exe"))) {
      if ((Test-Path -LiteralPath $candidate -PathType Leaf) -and (Test-Path -LiteralPath "$candidate.sha256" -PathType Leaf)) {
        $checksum = Get-Content -LiteralPath "$candidate.sha256" -Raw
        if ($checksum -match $hashPattern) {
            $expected = $Matches[1]
            if ((Get-FileHash -LiteralPath $candidate -Algorithm SHA256).Hash -eq $expected) {
                $cachedVersion = & $candidate --version 2>$null
                if ($LASTEXITCODE -ne 0 -or $cachedVersion -ne "bulbul $version") { throw 'Cached executable has an unexpected version.' }
                return $candidate
            }
        }
      }
    }

    try { [void][IO.Directory]::CreateDirectory($cacheDir) } catch {
        throw '程序缓存目录不可写。请在可写的项目目录运行，或设置 BULBUL_CACHE_DIR；AI 工具中还需允许该目录的访问。'
    }
    $partial = Join-Path $cacheDir ('.download.' + [guid]::NewGuid().ToString('N') + '.exe')
    try {
        $probe = [IO.File]::Open($partial, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write)
        $probe.Dispose()
    } catch {
        throw '程序缓存目录不可写。请在可写的项目目录运行，或设置 BULBUL_CACHE_DIR；AI 工具中还需允许该目录的访问。'
    }
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

function Invoke-Bulbul {
    param([string[]]$CliArgs)
    try {
        if ($CliArgs.Count -gt 0 -and $CliArgs[0] -eq 'list') {
            return (Show-BulbulTools -Names @($CliArgs | Select-Object -Skip 1))
        }
        $binary = Get-BulbulBinary
    } catch {
        [Console]::Error.WriteLine('Bulbul: ' + $_.Exception.Message)
        return 1
    }
    & $binary @CliArgs | ForEach-Object { [Console]::Out.WriteLine($_) }
    return $LASTEXITCODE
}

if ($MyInvocation.InvocationName -ne '.') {
    exit (Invoke-Bulbul -CliArgs @($args))
}
