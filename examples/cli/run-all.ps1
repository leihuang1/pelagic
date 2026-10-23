# Validate and run every example case with the command-line program (Windows PowerShell).
#
#   .\examples\cli\run-all.ps1 [-Output results\cli]
#
# Uses pelagic.exe on PATH, or the program named by $env:PELAGIC_CLI.
param([string]$Output = "results\cli")
$ErrorActionPreference = "Stop"
$pelagic = if ($env:PELAGIC_CLI) { $env:PELAGIC_CLI } else { "pelagic" }
$cases = Join-Path $PSScriptRoot "..\cases"

& $pelagic version
foreach ($json in Get-ChildItem -Path $cases -Filter *.json -Recurse | Where-Object { $_.Name -ne "catalog.json" }) {
    $model = $json.Directory.Parent.Name            # bellhop or kraken
    $target = Join-Path $Output "$model\$($json.BaseName)"
    & $pelagic $model validate $json.FullName | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "invalid case $($json.FullName)" }
    & $pelagic $model run $json.FullName --format both --output $target 2>$null
    if ($LASTEXITCODE -ne 0) { throw "run failed for $($json.FullName) (exit $LASTEXITCODE)" }
}
