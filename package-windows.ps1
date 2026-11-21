param(
    [string] $OutputPath = (Join-Path $PSScriptRoot 'target/Nhip-windows-x64.zip')
)

$ErrorActionPreference = 'Stop'
$manifest = Join-Path $PSScriptRoot 'Cargo.toml'
$target = 'x86_64-pc-windows-msvc'
$metadata = cargo metadata --locked --no-deps --format-version 1 --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$targetDirectory = ($metadata | ConvertFrom-Json).target_directory
cargo build --locked --release --target $target --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$releaseExe = Join-Path $targetDirectory "$target/release/nhip.exe"
$stage = Join-Path ([IO.Path]::GetTempPath()) ('nhip-zip-' + [guid]::NewGuid().ToString('N'))
$stagedExe = Join-Path $stage 'nhip.exe'
$marker = Join-Path $stage 'nhip.portable'
$kawarpLicense = Join-Path $stage 'LICENSE-KAWARP.txt'
$archive = Join-Path $stage 'nhip.zip'
$destination = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputPath)

New-Item -Path $stage -ItemType Directory | Out-Null
try {
    Copy-Item -LiteralPath $releaseExe -Destination $stagedExe
    New-Item -Path $marker -ItemType File | Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'src/ui/backdrop/LICENSE-KAWARP') -Destination $kawarpLicense
    Compress-Archive -LiteralPath $stagedExe, $marker, $kawarpLicense -DestinationPath $archive
    New-Item -Path ([IO.Path]::GetDirectoryName($destination)) -ItemType Directory -Force | Out-Null
    Move-Item -LiteralPath $archive -Destination $destination -Force
    Get-Item -LiteralPath $destination
} finally {
    foreach ($file in $archive, $marker, $stagedExe, $kawarpLicense) {
        if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file }
    }
    Remove-Item -LiteralPath $stage
}
