[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $Name,
    [Parameter(Mandatory = $true)] [string] $Engine,
    [Parameter(Mandatory = $true)] [string] $Level,
    [Parameter(Mandatory = $true)] [string] $Resources,
    [int] $Runs = 5,
    [double] $Time = 1.0,
    [string] $Renderer = "target/release/renderer.exe",
    [string] $OutputDirectory = "artifacts/sono-gcc-performance"
)

$ErrorActionPreference = "Stop"
$rendererPath = (Resolve-Path -LiteralPath $Renderer).Path
$enginePath = (Resolve-Path -LiteralPath $Engine).Path
$levelPath = (Resolve-Path -LiteralPath $Level).Path
$resourcesPath = (Resolve-Path -LiteralPath $Resources).Path
$outputPath = [System.IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null

$rows = [System.Collections.Generic.List[object]]::new()
foreach ($mode in @("interpreter", "sono-gcc")) {
    for ($run = 1; $run -le $Runs; $run++) {
        $logPath = Join-Path $outputPath "$Name-$mode-$run.log"
        $watch = [System.Diagnostics.Stopwatch]::StartNew()
        $lines = & $rendererPath run-watch $enginePath $levelPath `
            --resources $resourcesPath --time $Time --vm $mode `
            --no-ui --no-particles --no-sfx --no-bgm 2>&1
        $exitCode = $LASTEXITCODE
        $watch.Stop()
        $text = ($lines | ForEach-Object { "$_" }) -join [Environment]::NewLine
        Set-Content -LiteralPath $logPath -Value $text -Encoding utf8
        if ($exitCode -ne 0) {
            throw "$Name $mode run $run failed with exit code $exitCode. See $logPath"
        }

        $watchMs = $null
        if ($text -match 'Watch frame execution elapsed:\s+([0-9.]+)s') {
            $watchMs = [double]$Matches[1] * 1000.0
        }
        $compileMs = $null
        $cacheLookupMs = $null
        $dllLoadMs = $null
        if ($text -match 'compile\s+([0-9.]+)s, cache lookup\s+([0-9.]+)s, DLL load\s+([0-9.]+)s') {
            $compileMs = [double]$Matches[1] * 1000.0
            $cacheLookupMs = [double]$Matches[2] * 1000.0
            $dllLoadMs = [double]$Matches[3] * 1000.0
        }
        $callbacks = $null
        $draws = $null
        $regions = $null
        $cuts = $null
        $overflowInputs = $null
        $drawHash = $null
        $displayHash = $null
        if ($text -match 'callbacks:\s*(\d+), Draw commands:\s*(\d+)') {
            $callbacks = [long]$Matches[1]
            $draws = [long]$Matches[2]
        }
        if ($text -match 'Sono-GCC regions executed this frame:\s*(\d+)') {
            $regions = [long]$Matches[1]
        }
        if ($text -match 'Sono-GCC VM-to-native scalar cut calls this frame:\s*(\d+)') {
            $cuts = [long]$Matches[1]
        }
        if ($text -match 'Sono-GCC regions needing heap cut-input storage this frame:\s*(\d+)') {
            $overflowInputs = [long]$Matches[1]
        }
        if ($text -match 'raw Draw SHA-1:\s*([0-9a-fA-F]+)') {
            $drawHash = $Matches[1].ToLowerInvariant()
        }
        if ($text -match 'display-list SHA-256:\s*([0-9a-fA-F]+)') {
            $displayHash = $Matches[1].ToLowerInvariant()
        }

        $rows.Add([pscustomobject]@{
            Workload = $Name
            Mode = $mode
            Run = $run
            WatchMs = $watchMs
            ProcessMs = $watch.Elapsed.TotalMilliseconds
            CompileMs = $compileMs
            CacheLookupMs = $cacheLookupMs
            DllLoadMs = $dllLoadMs
            Callbacks = $callbacks
            Draws = $draws
            GccRegions = $regions
            VmToNativeCuts = $cuts
            HeapInputRegions = $overflowInputs
            DrawSha1 = $drawHash
            DisplaySha256 = $displayHash
            ExitCode = $exitCode
            Log = $logPath
        })
    }
}

$csvPath = Join-Path $outputPath "$Name.csv"
$rows | Export-Csv -LiteralPath $csvPath -NoTypeInformation
$rows | Format-Table Workload, Mode, Run, WatchMs, ProcessMs, CompileMs, CacheLookupMs, DllLoadMs, Callbacks, Draws, GccRegions, VmToNativeCuts, HeapInputRegions -AutoSize
Write-Output "Wrote $csvPath"
