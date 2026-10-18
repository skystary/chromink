param([Parameter(Mandatory=$true)][string]$LlvmMingwBin)
$ErrorActionPreference = 'Stop'
$taskCompilerBin = (Resolve-Path -LiteralPath $LlvmMingwBin).Path
foreach ($tool in @('llvm-dlltool.exe', 'windres.exe')) {
    if (!(Test-Path -LiteralPath (Join-Path $taskCompilerBin $tool))) { throw "Missing $tool in $taskCompilerBin" }
}
$taskRustRoot = (& rustc +stable-x86_64-pc-windows-gnu --print sysroot).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Install stable-x86_64-pc-windows-gnu first.' }
$taskLinker = Join-Path $taskRustRoot 'lib/rustlib/x86_64-pc-windows-gnu/bin/rust-lld.exe'
$taskOldPath = $env:PATH
$taskOldFlags = $env:CARGO_ENCODED_RUSTFLAGS
$taskOldRustFlags = $env:RUSTFLAGS
try {
    $env:PATH = "$taskRustRoot/bin;$taskCompilerBin;$taskOldPath"
    $env:RUSTFLAGS = $null
    $env:CARGO_ENCODED_RUSTFLAGS = @('-C', "dlltool=$(Join-Path $taskCompilerBin 'llvm-dlltool.exe')", '-C', "linker=$taskLinker", '-C', 'linker-flavor=ld.lld', '-C', 'link-self-contained=yes') -join [char]31
    $taskManifest = Join-Path $PSScriptRoot '../Cargo.toml'
    & cargo +stable-x86_64-pc-windows-gnu check --manifest-path $taskManifest --workspace --locked
    if ($LASTEXITCODE -ne 0) { throw 'Core check failed.' }
    & cargo +stable-x86_64-pc-windows-gnu build --manifest-path $taskManifest --workspace --release --features chromink-desktop/custom-protocol --locked
    if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
    Write-Output 'Distribute desktop executable and WebView2Loader.dll together from target/release.'
} finally {
    $env:PATH = $taskOldPath
    $env:CARGO_ENCODED_RUSTFLAGS = $taskOldFlags
    $env:RUSTFLAGS = $taskOldRustFlags
}
