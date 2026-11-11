param(
    [string]$Workspace = $env:GOV_WORKSPACE,
    [string]$Mode = $env:GOV_SETUP_MODE,
    [string]$Python = $env:GOV_PYTHON,
    [string]$TorchIndexUrl = $env:GOV_TORCH_INDEX_URL
)

& {
    $ErrorActionPreference = 'Stop'
    if (-not $Workspace) {
        throw 'Specify -Workspace or set GOV_WORKSPACE before running the installer.'
    }
    if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64' -or
        -not [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
            [System.Runtime.InteropServices.OSPlatform]::Windows)) {
        throw 'This installer supports Windows x86-64.'
    }

    $archive = 'gov-x86_64-pc-windows-msvc.zip'
    $base = 'https://github.com/SinVC/comfy_env/releases/download/@RELEASE_TAG@'
    $stage = New-Item -ItemType Directory -Path (Join-Path ([IO.Path]::GetTempPath()) ("gov-" + [guid]::NewGuid()))
    try {
        $package = Join-Path $stage.FullName $archive
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$archive" -OutFile $package
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$archive.sha256" -OutFile "$package.sha256"
        $expected = (Get-Content -LiteralPath "$package.sha256" -Raw).Trim()
        $actual = (Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash
        if ($actual -ne $expected) {
            throw "SHA-256 mismatch for $archive"
        }
        Expand-Archive -LiteralPath $package -DestinationPath $stage.FullName
        $setupArgs = @('setup', '--workspace', $Workspace)
        if ($Mode) { $setupArgs += @('--mode', $Mode) }
        if ($Python) { $setupArgs += @('--python', $Python) }
        if ($TorchIndexUrl) { $setupArgs += @('--torch-index-url', $TorchIndexUrl) }
        & (Join-Path $stage.FullName 'gov.exe') @setupArgs
        if ($LASTEXITCODE -ne 0) {
            throw "gov setup failed with exit code $LASTEXITCODE"
        }
    } finally {
        Remove-Item -LiteralPath $stage.FullName -Recurse -Force
    }
}
