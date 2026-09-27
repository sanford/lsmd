# Build lsmd, install it to ~\.local\bin, and run it. The Windows twin of run.sh.
# Any arguments are passed through: .\run.ps1 README.md, .\run.ps1 -p x.md, ...
$ErrorActionPreference = 'Stop'

Set-Location $PSScriptRoot
$binDir = Join-Path $HOME '.local\bin'

cargo build --release --quiet
if ($LASTEXITCODE) { exit $LASTEXITCODE }
New-Item -ItemType Directory -Force $binDir | Out-Null
Copy-Item target\release\lsmd.exe $binDir -Force

& (Join-Path $binDir 'lsmd.exe') @args
exit $LASTEXITCODE
