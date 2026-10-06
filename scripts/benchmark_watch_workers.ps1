param(
    [string]$RendererPath = "target\cpu-followup5-build\release\renderer.exe",
    [int]$Iterations = 3,
    [ValidateSet("Larp", "ARMAGEDDON", "Baumkuchen")]
    [string[]]$Workloads = @("Larp", "ARMAGEDDON"),
    [ValidateSet("single", "auto", "2", "4", "8", "12")]
    [string[]]$Modes = @("single", "auto", "2", "4", "8", "12"),
    [switch]$KeepVmAccounting,
    [switch]$CollectProfile,
    [string]$OutputPath = "artifacts\sono-renderer-followup5\watch-worker-benchmarks.json"
)

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$rendererFullPath = (Resolve-Path (Join-Path $repoRoot $RendererPath)).Path
$engine = Join-Path $repoRoot "TestingSuite\Next RUSH\engine\Next RUSH.zip"
$resources = Join-Path $repoRoot "TestingSuite\Next Sekai Engine\levels\larp 64x\ProSeka Faithful 0.8.4.scp"
$workloadInputs = @{
    Larp = Join-Path $repoRoot "TestingSuite\Next Sekai Engine\levels\larp 64x\larp 64x.json.gz"
    ARMAGEDDON = Join-Path $repoRoot "TestingSuite\Next Sekai Engine\levels\Touhou - ARMAGEDDON\Armageddon.json.gz"
    Baumkuchen = Join-Path $repoRoot "TestingSuite\Next Sekai Engine\levels\Various Artists - Baumkuchen x Retry Now\Baumkuchen x Retry Now.json.gz"
}

if ($Iterations -lt 1) {
    throw "Iterations must be at least one."
}
foreach ($path in @($engine, $resources) + @($Workloads | ForEach-Object { $workloadInputs[$_] })) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required fixture does not exist: $path"
    }
}

$rows = [System.Collections.Generic.List[object]]::new()
foreach ($workload in $Workloads) {
    foreach ($mode in $Modes) {
        for ($iteration = 1; $iteration -le $Iterations; $iteration++) {
            $arguments = [System.Collections.Generic.List[string]]::new()
            foreach ($argument in @(
                "run-watch", $engine, $workloadInputs[$workload],
                "--resources", $resources, "--time", "1",
                "--no-ui", "--no-particles", "--no-sfx", "--no-bgm"
            )) {
                $arguments.Add([string]$argument)
            }
            if (-not $KeepVmAccounting) {
                $arguments.Add("--no-vm-accounting")
            }
            switch ($mode) {
                "single" { $arguments.Add("--single-threaded") }
                "auto" { }
                default { $arguments.Add("--watch-workers"); $arguments.Add($mode) }
            }
            if ($CollectProfile) {
                $arguments.Add("--profile")
            }

            $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
            $startInfo.FileName = $rendererFullPath
            $startInfo.WorkingDirectory = $repoRoot
            $startInfo.UseShellExecute = $false
            $startInfo.CreateNoWindow = $true
            $startInfo.RedirectStandardOutput = $true
            $startInfo.RedirectStandardError = $true
            $startInfo.Arguments = ($arguments | ForEach-Object {
                '"' + $_.Replace('"', '\"') + '"'
            }) -join " "

            $process = [System.Diagnostics.Process]::new()
            $process.StartInfo = $startInfo
            $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
            if (-not $process.Start()) {
                throw "Could not start renderer: $rendererFullPath"
            }
            $stdoutTask = $process.StandardOutput.ReadToEndAsync()
            $stderrTask = $process.StandardError.ReadToEndAsync()
            $peakWorkingSetBytes = 0L
            while (-not $process.WaitForExit(25)) {
                try {
                    $peakWorkingSetBytes = [math]::Max($peakWorkingSetBytes, $process.WorkingSet64)
                }
                catch {
                    # The process may exit between the poll and the memory sample.
                }
            }
            $stopwatch.Stop()
            $stdout = $stdoutTask.GetAwaiter().GetResult()
            $stderr = $stderrTask.GetAwaiter().GetResult()
            $cpuMilliseconds = $process.TotalProcessorTime.TotalMilliseconds
            try {
                $peakWorkingSetBytes = [math]::Max($peakWorkingSetBytes, $process.WorkingSet64)
            }
            catch {
                # The process handle may no longer expose its final working set.
            }
            $peakWorkingSetMegabytes = $peakWorkingSetBytes / 1MB
            $exitCode = $process.ExitCode
            $process.Dispose()
            if ($exitCode -ne 0) {
                throw "Renderer failed ($workload, $mode, iteration $iteration; exit $exitCode):`n$stderr`n$stdout"
            }

            $frameMatch = [regex]::Match($stdout, "Watch frame execution elapsed: ([0-9.]+)s")
            $workMatch = [regex]::Match($stdout, "runtime entities: (\d+), active entities: (\d+), callbacks: (\d+), Draw commands: (\d+)")
            $hashMatch = [regex]::Match($stdout, "raw Draw SHA-1: ([0-9a-fA-F]+)")
            if (-not $frameMatch.Success -or -not $workMatch.Success -or -not $hashMatch.Success) {
                throw "Renderer output was missing benchmark fields ($workload, $mode):`n$stdout"
            }
            $profileMatch = [regex]::Match($stdout, "(?m)^Watch CPU profile: (.+)$")
            $row = [pscustomobject]@{
                workload = $workload
                mode = $mode
                iteration = $iteration
                frame_seconds = [double]::Parse($frameMatch.Groups[1].Value, [Globalization.CultureInfo]::InvariantCulture)
                process_seconds = $stopwatch.Elapsed.TotalSeconds
                process_cpu_milliseconds = $cpuMilliseconds
                peak_working_set_megabytes = [math]::Round($peakWorkingSetMegabytes, 2)
                vm_accounting_enabled = [bool]$KeepVmAccounting
                runtime_entities = [int]$workMatch.Groups[1].Value
                active_entities = [int]$workMatch.Groups[2].Value
                callbacks = [int]$workMatch.Groups[3].Value
                draw_commands = [int]$workMatch.Groups[4].Value
                draw_sha1 = $hashMatch.Groups[1].Value.ToLowerInvariant()
                profile = if ($profileMatch.Success) { $profileMatch.Groups[1].Value } else { $null }
            }
            $rows.Add($row)
            Write-Host ("{0,-12} {1,-6} run {2}: frame {3,7:N3}s, process {4,7:N3}s, CPU {5,7:N0}ms, peak {6,7:N1}MB, callbacks {7,6}, draws {8,6}, hash {9}" -f `
                $workload, $mode, $iteration, $row.frame_seconds, $row.process_seconds,
                $row.process_cpu_milliseconds, $row.peak_working_set_megabytes,
                $row.callbacks, $row.draw_commands, $row.draw_sha1)
        }
    }
}

$outputFullPath = Join-Path $repoRoot $OutputPath
$outputDirectory = Split-Path -Parent $outputFullPath
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
$rows | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $outputFullPath -Encoding utf8
Write-Host "Saved benchmark rows to $outputFullPath"
