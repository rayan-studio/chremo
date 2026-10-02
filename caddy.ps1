# Invoke Python directly so URL characters never pass through cmd.exe.
$python = Join-Path $PSScriptRoot '.venv\Scripts\python.exe'
if (-not (Test-Path -LiteralPath $python)) {
    $python = 'python'
}
& $python (Join-Path $PSScriptRoot 'support\local_build.py') @args
exit $LASTEXITCODE
