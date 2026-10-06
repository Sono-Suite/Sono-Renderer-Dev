param(
    [switch]$Apply
)

$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$targetRoot = [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot 'target'))
$allowedBuildDirectories = @(
    'debug',
    'release',
    'base-clear-original',
    'base-clear-history-9f'
)

foreach ($name in $allowedBuildDirectories) {
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $targetRoot $name))
    $expected = [System.IO.Path]::GetFullPath((Join-Path $targetRoot $name))
    if (-not [string]::Equals($candidate, $expected, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing unexpected cleanup path: $candidate"
    }

    if (-not (Test-Path -LiteralPath $candidate -PathType Container)) {
        Write-Output "Already absent: target/$name"
        continue
    }

    $item = Get-Item -LiteralPath $candidate -Force
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Refusing to follow reparse point: $candidate"
    }

    if ($name -eq 'release') {
        $rendererPath = [System.IO.Path]::GetFullPath((Join-Path $candidate 'renderer.exe'))
        $runningRenderers = @(Get-Process -Name 'renderer' -ErrorAction SilentlyContinue |
            Where-Object {
                try {
                    $_.Path -and [string]::Equals(
                        [System.IO.Path]::GetFullPath($_.Path),
                        $rendererPath,
                        [System.StringComparison]::OrdinalIgnoreCase
                    )
                } catch {
                    $false
                }
            })
        if ($runningRenderers.Count -gt 0) {
            $ids = ($runningRenderers | ForEach-Object Id) -join ', '
            Write-Output "Skipping target/release; renderer.exe is running (PID $ids)."
            continue
        }
    }

    if ($name -like 'base-clear-*' -and
        (-not (Test-Path -LiteralPath (Join-Path $candidate '.rustc_info.json') -PathType Leaf) -or
         -not (Test-Path -LiteralPath (Join-Path $candidate 'release') -PathType Container))) {
        throw "Refusing unexpected custom Cargo target layout: $candidate"
    }

    if ($Apply) {
        Remove-Item -LiteralPath $candidate -Recurse -Force
        Write-Output "Removed generated Cargo output: target/$name"
    } else {
        $bytes = (Get-ChildItem -LiteralPath $candidate -File -Recurse -Force |
            Measure-Object -Property Length -Sum).Sum
        Write-Output ("Would remove target/{0} ({1:N2} GiB). Pass -Apply to delete." -f $name, ($bytes / 1GB))
    }
}

Write-Output 'Diagnostic and test evidence outside these four build directories is preserved.'
