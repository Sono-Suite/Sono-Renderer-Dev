param(
    [string]$Renderer = 'target\release\renderer.exe',
    [string]$Engine = 'TestingSuite\Next Sekai Engine\levels\larp 64x\Next RUSH.zip',
    [string]$Resources = 'TestingSuite\Next Sekai Engine\levels\larp 64x\ProSeka Faithful 0.8.4.scp',
    [string]$Level = 'TestingSuite\Next Sekai Engine\levels\larp 64x\larp 64x.json.gz',
    [string]$Music = 'TestingSuite\Next Sekai Engine\levels\larp 64x\larp 64x.mp3',
    [int]$Duration = 2,
    [int]$Fps = 12,
    [int]$Width = 3840,
    [int]$Height = 2160,
    [int[]]$Workers = @(1, 2, 3, 4),
    [int]$Runs = 3,
    [string]$OutputDirectory = 'artifacts\sono-renderer-follow-up4\repeat'
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$outputRoot = Join-Path $repo $OutputDirectory
New-Item -ItemType Directory -Force $outputRoot | Out-Null

function Quote-ProcessArgument([string]$Value) {
    '"' + $Value.Replace('"', '\"') + '"'
}

$rendererPath = (Resolve-Path (Join-Path $repo $Renderer)).Path
$argumentsBase = @(
    'render-video',
    (Join-Path $repo $Engine),
    (Join-Path $repo $Resources),
    (Join-Path $repo $Level),
    (Join-Path $repo $Music),
    $null,
    '--start-time', '0',
    '--duration', $Duration.ToString([Globalization.CultureInfo]::InvariantCulture),
    '--fps', $Fps.ToString([Globalization.CultureInfo]::InvariantCulture),
    '--width', $Width.ToString([Globalization.CultureInfo]::InvariantCulture),
    '--height', $Height.ToString([Globalization.CultureInfo]::InvariantCulture),
    '--backend', 'wgpu',
    '--vm', 'sono-gcc',
    '--profile'
)

$oldWorkerCount = $env:SONO_WGPU_IN_FLIGHT
$summary = [System.Collections.Generic.List[object]]::new()
try {
    foreach ($workerCount in $Workers) {
        if ($workerCount -lt 1 -or $workerCount -gt 4) {
            throw "Worker count must be between 1 and 4; got $workerCount"
        }
        for ($run = 1; $run -le $Runs; $run++) {
            $tag = "w$workerCount-r$run"
            $videoPath = Join-Path $outputRoot "$tag.mp4"
            $stdoutPath = Join-Path $outputRoot "$tag.stdout.log"
            $stderrPath = Join-Path $outputRoot "$tag.stderr.log"
            $arguments = $argumentsBase.Clone()
            $arguments[5] = $videoPath
            $argumentLine = ($arguments | ForEach-Object { Quote-ProcessArgument ([string]$_) }) -join ' '
            $env:SONO_WGPU_IN_FLIGHT = $workerCount.ToString([Globalization.CultureInfo]::InvariantCulture)
            $timer = [System.Diagnostics.Stopwatch]::StartNew()
            $process = Start-Process `
                -FilePath $rendererPath `
                -ArgumentList $argumentLine `
                -WindowStyle Hidden `
                -RedirectStandardOutput $stdoutPath `
                -RedirectStandardError $stderrPath `
                -PassThru
            $peakWorkingSet = 0L
            while (-not $process.WaitForExit(50)) {
                try {
                    $process.Refresh()
                    $peakWorkingSet = [math]::Max($peakWorkingSet, $process.WorkingSet64)
                } catch {
                    # The process can exit between WaitForExit and Refresh.
                }
            }
            $process.Refresh()
            try { $peakWorkingSet = [math]::Max($peakWorkingSet, $process.WorkingSet64) } catch {}
            $timer.Stop()
            $exitCode = $process.ExitCode
            $stderrText = Get-Content -Raw $stderrPath
            $stdoutText = Get-Content -Raw $stdoutPath
            if (($null -ne $exitCode) -and ($exitCode -ne 0)) {
                throw "Renderer failed (workers=$workerCount run=$run, exit=$($process.ExitCode)); see $stderrPath"
            }
            $expectedFrames = $Duration * $Fps
            if (!(Test-Path $videoPath) -or (Get-Item $videoPath).Length -eq 0 -or
                $stderrText -notmatch 'Total export:' -or
                $stderrText -notmatch 'FFprobe validation' -or
                $stdoutText -notmatch "submitted deterministic frames: $expectedFrames" -or
                $stdoutText -notmatch 'SFX mixed: true') {
                throw "Renderer did not produce a validated export (workers=$workerCount run=$run); see $stdoutPath and $stderrPath"
            }
            $summary.Add([pscustomobject]@{
                workers = $workerCount
                run = $run
                wall_seconds = [math]::Round($timer.Elapsed.TotalSeconds, 3)
                sampled_peak_working_set_mb = [math]::Round($peakWorkingSet / 1MB, 1)
                video = $videoPath
                stdout_log = $stdoutPath
                stderr_log = $stderrPath
            })
            $process.Dispose()
        }
    }
}
finally {
    if ($null -eq $oldWorkerCount) {
        Remove-Item Env:\SONO_WGPU_IN_FLIGHT -ErrorAction SilentlyContinue
    } else {
        $env:SONO_WGPU_IN_FLIGHT = $oldWorkerCount
    }
}

$summaryPath = Join-Path $outputRoot 'summary.csv'
$summary | Export-Csv -NoTypeInformation -Encoding UTF8 $summaryPath
$summary | Format-Table -AutoSize
Write-Output "Saved benchmark summary to $summaryPath"
