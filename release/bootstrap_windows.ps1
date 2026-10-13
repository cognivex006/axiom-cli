#requires -version 5.1
[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$Root,[switch]$DryRun,[switch]$Apply,[string]$ApproveDigest)
$ErrorActionPreference='Stop'
Set-StrictMode -Version 2
if ($DryRun -eq $Apply) { throw 'choose exactly one execution mode' }
$release = $PSScriptRoot
$target = [IO.Path]::GetFullPath($Root)
if ((Test-Path -LiteralPath $target) -and (Get-ChildItem -LiteralPath $target -Force)) { throw 'fresh empty root required; existing installations use CLI update' }
function Digest([string]$path) { (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() }
$inventory = Get-Content -LiteralPath (Join-Path $release 'DISTRIBUTION-FILES.json') -Raw | ConvertFrom-Json
foreach ($row in $inventory.files) {
    $path = Join-Path $release $row.path
    if ((Digest $path) -cne $row.sha256) { throw 'distribution bytes changed' }
}
$seal = $target + "`n" + (Digest (Join-Path $release 'DISTRIBUTION-FILES.json'))
$hasher=[Security.Cryptography.SHA256]::Create()
try { $digest=[BitConverter]::ToString($hasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($seal))).Replace('-','').ToLowerInvariant() } finally { $hasher.Dispose() }
$releaseVersion = (Get-Content -LiteralPath (Join-Path $release 'release-set.json') -Raw | ConvertFrom-Json).release_version
if ($releaseVersion -cnotmatch '^[0-9]+\.[0-9]+\.[0-9]+$') { throw 'invalid pinned release version' }
$channel = Get-Content -LiteralPath (Join-Path $release 'channel.json') -Raw | ConvertFrom-Json
$mcpEntries = @($channel.components | Where-Object { $_.component -eq 'axiom-mcp' })
if ($mcpEntries.Count -ne 1) { throw 'one pinned MCP component required' }
$mcpVersion = $mcpEntries[0].version
if ($mcpVersion -cnotmatch '^[0-9]+\.[0-9]+\.[0-9]+$') { throw 'invalid pinned MCP version' }
if ($DryRun) { @{status='planned';version=$releaseVersion;plan_digest=$digest} | ConvertTo-Json -Compress; exit 0 }
if ($ApproveDigest -cne $digest) { throw 'exact reviewed distribution approval required' }
$runtimeInputs = Get-Content -LiteralPath (Join-Path $release 'runtime-input.json') -Raw | ConvertFrom-Json
$runtime = & (Join-Path $release 'Provision-McpRuntime.ps1') -Action provision -Root (Join-Path $target 'mcp-runtime') `
    -RuntimeArchive (Join-Path $release 'python-3.13.15-embed-amd64.zip') -RuntimeSha256 $runtimeInputs.python.sha256 `
    -PipWheel (Join-Path $release 'pip-26.1.2-py3-none-any.whl') -PipSha256 $runtimeInputs.pip.sha256 `
    -Inputs (Join-Path $release 'windows-x64-py313-inputs.tar') -InputsSha256 $runtimeInputs.mcp_inputs.sha256 `
    -SourceRevision $runtimeInputs.mcp_inputs.source_revision -Version $mcpVersion
if ($LASTEXITCODE -ne 0) { throw 'runtime provision failed' }
$env:AXIOM_HOME=$target
$env:AXIOM_CLI_INSTALL_ROOT=$target
$cli=Join-Path $release 'axiom-cli.exe'
$plan = (& $cli install --from $release --dry-run --json | ConvertFrom-Json)
if ($LASTEXITCODE -ne 0) { throw 'engine planning failed' }
$approval=$plan.details.plan_digest
if (-not $approval) { throw 'engine approval missing' }
& $cli install --from $release --apply --approve-digest $approval --json
if ($LASTEXITCODE -ne 0) { throw 'engine apply failed' }
