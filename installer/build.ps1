# Builds Charpick and its installer: dist\Charpick-Setup.exe
#   powershell -NoProfile -ExecutionPolicy Bypass -File installer\build.ps1

$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
Push-Location $root
try {
    if (-not (Test-Path 'assets\charpick.ico')) {
        & powershell -NoProfile -ExecutionPolicy Bypass -File tools\make-icons.ps1
    }

    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

    # Find the Inno Setup compiler (6 or 7, machine-wide or per-user install).
    $iscc = (Get-Command iscc.exe -ErrorAction SilentlyContinue).Source
    if (-not $iscc) {
        $candidates = @()
        foreach ($base in @($env:ProgramFiles, ${env:ProgramFiles(x86)}, "$env:LOCALAPPDATA\Programs")) {
            if ($base) { $candidates += Get-ChildItem -Path $base -Filter 'Inno Setup*' -Directory -ErrorAction SilentlyContinue }
        }
        $iscc = $candidates | ForEach-Object { Join-Path $_.FullName 'ISCC.exe' } | Where-Object { Test-Path $_ } |
            Sort-Object -Descending | Select-Object -First 1
    }
    if (-not $iscc) { throw 'Inno Setup was not found. Install it from https://jrsoftware.org/isdl.php' }

    & $iscc /Q 'installer\charpick.iss'
    if ($LASTEXITCODE -ne 0) { throw 'ISCC failed' }
    Write-Host "Built $(Resolve-Path 'dist\Charpick-Setup.exe')"
}
finally {
    Pop-Location
}
