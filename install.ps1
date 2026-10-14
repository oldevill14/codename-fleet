# codename installer (Windows, no admin needed)
#
#   irm https://raw.githubusercontent.com/oldevill14/codename-fleet/main/install.ps1 | iex
#
# Downloads the latest release zip, verifies its SHA256, installs codename.exe
# and a copy named cn.exe into %USERPROFILE%\.local\bin, and adds that folder
# to the user PATH if missing. Existing exe files are renamed to *.bak, never
# deleted. Works on Windows PowerShell 5.1 and PowerShell 7.

# Everything runs inside a script block so nothing leaks into the caller's
# session when piped through `irm | iex`.
& {
    $ErrorActionPreference = 'Stop'
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $repo    = 'oldevill14/codename-fleet'
    $asset   = 'codename-x86_64-pc-windows-msvc.zip'
    $binDir  = Join-Path $env:USERPROFILE '.local\bin'
    $stamp   = Get-Date -Format 'yyyyMMdd-HHmmss'
    $work    = Join-Path $env:TEMP ("codename-install-" + $stamp)

    Write-Host "codename installer - repo $repo"

    $release = Invoke-RestMethod -UseBasicParsing -Uri "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'codename-installer' }
    $zipAsset = $release.assets | Where-Object { $_.name -eq $asset } | Select-Object -First 1
    $shaAsset = $release.assets | Where-Object { $_.name -eq ($asset + '.sha256') } | Select-Object -First 1
    if (-not $zipAsset -or -not $shaAsset) {
        throw "Release $($release.tag_name) has no $asset (+ .sha256). Build from source instead: cargo build --release"
    }

    New-Item -ItemType Directory -Force -Path $work | Out-Null
    $zipPath = Join-Path $work $asset
    $shaPath = Join-Path $work ($asset + '.sha256')
    Invoke-WebRequest -UseBasicParsing -Uri $zipAsset.browser_download_url -OutFile $zipPath
    Invoke-WebRequest -UseBasicParsing -Uri $shaAsset.browser_download_url -OutFile $shaPath

    $expected = ((Get-Content -Raw $shaPath).Trim() -split '\s+')[0].ToLower()
    $actual   = (Get-FileHash -Algorithm SHA256 $zipPath).Hash.ToLower()
    if ($expected -ne $actual) {
        throw "SHA256 mismatch! expected $expected, got $actual. Nothing was installed."
    }
    Write-Host "SHA256 ok ($actual)"

    $unpack = Join-Path $work 'unpack'
    Expand-Archive -Path $zipPath -DestinationPath $unpack -Force
    $exe = Get-ChildItem -Path $unpack -Recurse -Filter 'codename.exe' | Select-Object -First 1
    if (-not $exe) { throw "codename.exe not found inside $asset" }

    New-Item -ItemType Directory -Force -Path $binDir | Out-Null
    foreach ($name in @('codename.exe', 'cn.exe')) {
        $dest = Join-Path $binDir $name
        if (Test-Path $dest) {
            $bak = $dest + '.' + $stamp + '.bak'
            Move-Item -Path $dest -Destination $bak
            Write-Host "kept previous $name as $bak"
        }
        Copy-Item -Path $exe.FullName -Destination $dest
    }

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $userPath) { $userPath = '' }
    $parts = $userPath -split ';' | Where-Object { $_ -ne '' }
    $present = $false
    foreach ($p in $parts) {
        if ($p.TrimEnd('\') -ieq $binDir.TrimEnd('\')) { $present = $true }
    }
    if (-not $present) {
        $newPath = (($parts + $binDir) -join ';')
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        Write-Host "added $binDir to your user PATH (open a new terminal)"
    }
    $env:Path = $env:Path + ';' + $binDir

    Write-Host ""
    Write-Host "Installed codename $($release.tag_name) to $binDir (also as cn.exe)."
    Write-Host "Temporary files are in $work (safe to remove yourself)."
    Write-Host ""
    Write-Host "Next steps (in a NEW terminal, with the Orca app open and its CLI enabled):"
    Write-Host "  setx ORCA_TELEMETRY_DISABLED 1"
    Write-Host "  codename serve-task install   # optional: headless Orca runtime at logon (no GUI)"
    Write-Host "  codename setup"
    Write-Host "  codename doctor"
}
