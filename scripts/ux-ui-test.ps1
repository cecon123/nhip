<#
.SYNOPSIS
    Automated 3-layer UX/UI test & visual regression runner for Nhip.
    Layer 1: cargo nextest (unit, shortcut, font, icon, headless layout tests)
    Layer 2: Deterministic offline demo screenshot capture across viewports
    Layer 3: Fast pixel-diff comparison against tests/ui-baselines/*.png + diff heatmaps

.EXAMPLE
    .\scripts\ux-ui-test.ps1                  # Run full UX/UI suite & compare against baselines
    .\scripts\ux-ui-test.ps1 -Quick           # Fast 3-screen smoke suite (1304x724 + 800x600 + lyrics)
    .\scripts\ux-ui-test.ps1 -UpdateBaselines # Capture and approve new baseline screenshots
#>
param(
    [switch] $Quick,
    [switch] $UpdateBaselines,
    [switch] $SkipUnitTests,
    [double] $MaxDiffPercent = 0.5,
    [int]    $ChannelTolerance = 4,
    [int]    $CaptureTimeoutSec = 30
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root

try {
    # -------------------------------------------------------------------------
    # Layer 1: Logic, shortcut, font, icon & headless egui layout tests
    # -------------------------------------------------------------------------
    if (-not $SkipUnitTests) {
        Write-Host "`n[1/3] Running workspace tests (cargo nt --locked)..." -ForegroundColor Cyan
        cargo nt --locked
        if ($LASTEXITCODE -ne 0) {
            throw "Workspace unit/layout tests failed (exit $LASTEXITCODE)."
        }
    }

    # -------------------------------------------------------------------------
    # Build debug binary once (uses rust-lld)
    # -------------------------------------------------------------------------
    Write-Host "`n[2/3] Building nhip binary (cargo build --locked)..." -ForegroundColor Cyan
    cargo build --locked
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build --locked failed (exit $LASTEXITCODE)."
    }

    $exe = Join-Path $root 'target\debug\nhip.exe'
    if (-not (Test-Path -LiteralPath $exe)) {
        throw "Built executable not found at $exe"
    }

    # -------------------------------------------------------------------------
    # Layer 2: Deterministic screenshot matrix
    # -------------------------------------------------------------------------
    $cases = if ($Quick) {
        @(
            @{ Name = 'home-1304x724';   Page = $null;     Size = '1304x724' },
            @{ Name = 'home-800x600';    Page = $null;     Size = '800x600'  },
            @{ Name = 'lyrics-1304x724'; Page = 'lyrics';  Size = '1304x724' }
        )
    } else {
        @(
            @{ Name = 'home-1304x724';     Page = $null;      Size = '1304x724' },
            @{ Name = 'home-800x600';      Page = $null;      Size = '800x600'  },
            @{ Name = 'search-1304x724';   Page = 'search';   Size = '1304x724' },
            @{ Name = 'album-1304x724';    Page = 'album';    Size = '1304x724' },
            @{ Name = 'artist-1304x724';   Page = 'artist';   Size = '1304x724' },
            @{ Name = 'playlist-1304x724'; Page = 'playlist'; Size = '1304x724' },
            @{ Name = 'queue-1304x724';    Page = 'queue';    Size = '1304x724' },
            @{ Name = 'lyrics-1304x724';   Page = 'lyrics';   Size = '1304x724' },
            @{ Name = 'settings-1304x724'; Page = 'settings'; Size = '1304x724' }
        )
    }

    $outDir      = Join-Path $root 'target\ui-qa\captures'
    $diffDir     = Join-Path $root 'target\ui-qa\diffs'
    $baselineDir = Join-Path $root 'tests\ui-baselines'
    $reportPath  = Join-Path $root 'target\ui-qa\report.json'

    New-Item -Path $outDir, $diffDir, $baselineDir -ItemType Directory -Force | Out-Null

    # Fast C# pixel comparator using System.Drawing.Bitmap.LockBits
    Add-Type -AssemblyName System.Drawing
    if (-not ('NhipVisualDiff' -as [type])) {
        Add-Type -ReferencedAssemblies 'System.Drawing' -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public class NhipDiffResult {
    public int Width;
    public int Height;
    public int BaseWidth;
    public int BaseHeight;
    public bool SizeMismatch;
    public bool IsBlank;
    public long TotalPixels;
    public long DiffPixels;
    public double DiffPercent;
}

public static class NhipVisualDiff {
    private static byte[] LoadRgba(Bitmap bmp) {
        var rect = new Rectangle(0, 0, bmp.Width, bmp.Height);
        var data = bmp.LockBits(rect, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        try {
            int bytes = Math.Abs(data.Stride) * bmp.Height;
            byte[] buf = new byte[bytes];
            Marshal.Copy(data.Scan0, buf, 0, bytes);
            return buf;
        } finally {
            bmp.UnlockBits(data);
        }
    }

    public static bool CheckBlank(string path, out int w, out int h) {
        using (var bmp = new Bitmap(path)) {
            w = bmp.Width;
            h = bmp.Height;
            var buf = LoadRgba(bmp);
            if (buf.Length < 4) return true;
            byte b0 = buf[0], g0 = buf[1], r0 = buf[2];
            for (int i = 4; i < buf.Length; i += 16) {
                if (Math.Abs(buf[i] - b0) > 2 || Math.Abs(buf[i + 1] - g0) > 2 || Math.Abs(buf[i + 2] - r0) > 2) {
                    return false;
                }
            }
            return true;
        }
    }

    public static NhipDiffResult Compare(string actualPath, string basePath, string diffOutPath, int tol) {
        var res = new NhipDiffResult();
        using (var act = new Bitmap(actualPath))
        using (var bas = new Bitmap(basePath)) {
            res.Width = act.Width;
            res.Height = act.Height;
            res.BaseWidth = bas.Width;
            res.BaseHeight = bas.Height;
            res.TotalPixels = (long)act.Width * act.Height;

            if (act.Width != bas.Width || act.Height != bas.Height) {
                res.SizeMismatch = true;
                res.DiffPixels = res.TotalPixels;
                res.DiffPercent = 100.0;
                return res;
            }

            var aBuf = LoadRgba(act);
            var bBuf = LoadRgba(bas);
            var dBuf = new byte[aBuf.Length];
            long diff = 0;

            bool allSame = true;
            byte ab0 = aBuf[0], ag0 = aBuf[1], ar0 = aBuf[2];

            for (int i = 0; i < aBuf.Length; i += 4) {
                byte ab = aBuf[i], ag = aBuf[i + 1], ar = aBuf[i + 2];
                byte bb = bBuf[i], bg = bBuf[i + 1], br = bBuf[i + 2];

                if (allSame && (Math.Abs(ab - ab0) > 2 || Math.Abs(ag - ag0) > 2 || Math.Abs(ar - ar0) > 2)) {
                    allSame = false;
                }

                bool changed = Math.Abs(ab - bb) > tol || Math.Abs(ag - bg) > tol || Math.Abs(ar - br) > tol;
                if (changed) {
                    diff++;
                    dBuf[i]     = 48;   // B
                    dBuf[i + 1] = 48;   // G
                    dBuf[i + 2] = 255;  // R (highlight)
                    dBuf[i + 3] = 255;  // A
                } else {
                    // Dimmed background for context
                    dBuf[i]     = (byte)(ab / 3);
                    dBuf[i + 1] = (byte)(ag / 3);
                    dBuf[i + 2] = (byte)(ar / 3);
                    dBuf[i + 3] = 255;
                }
            }

            res.IsBlank = allSame;
            res.DiffPixels = diff;
            res.DiffPercent = res.TotalPixels > 0 ? (diff * 100.0 / res.TotalPixels) : 0.0;

            if (diff > 0 && !string.IsNullOrEmpty(diffOutPath)) {
                using (var diffBmp = new Bitmap(act.Width, act.Height, PixelFormat.Format32bppArgb)) {
                    var rect = new Rectangle(0, 0, act.Width, act.Height);
                    var data = diffBmp.LockBits(rect, ImageLockMode.WriteOnly, PixelFormat.Format32bppArgb);
                    try {
                        Marshal.Copy(dBuf, 0, data.Scan0, dBuf.Length);
                    } finally {
                        diffBmp.UnlockBits(data);
                    }
                    diffBmp.Save(diffOutPath, ImageFormat.Png);
                }
            }
        }
        return res;
    }
}
'@
    }

    Write-Host "`n[3/3] Capturing & verifying $($cases.Count) UX/UI screen(s)..." -ForegroundColor Cyan
    $results = @()
    $failures = 0

    foreach ($c in $cases) {
        $name = $c.Name
        $shotPath = Join-Path $outDir "$name.png"
        $basePath = Join-Path $baselineDir "$name.png"
        $diffPath = Join-Path $diffDir "$name-diff.png"

        if (Test-Path -LiteralPath $shotPath) { Remove-Item -LiteralPath $shotPath -Force }
        if (Test-Path -LiteralPath $diffPath) { Remove-Item -LiteralPath $diffPath -Force }

        $argParts = @('--size', $c.Size, '--screenshot', ('"{0}"' -f $shotPath))
        if ($c.Page) {
            $argParts += @('--page', $c.Page)
        }

        $psi = New-Object System.Diagnostics.ProcessStartInfo
        $psi.FileName = $exe
        $psi.Arguments = ($argParts -join ' ')
        $psi.WorkingDirectory = $root
        $psi.UseShellExecute = $false

        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $proc = [System.Diagnostics.Process]::Start($psi)
        $exited = $proc.WaitForExit($CaptureTimeoutSec * 1000)
        $sw.Stop()

        if (-not $exited) {
            try { $proc.Kill() } catch {}
            Write-Host ("  [FAIL] {0,-20} Timed out after {1}s" -f $name, $CaptureTimeoutSec) -ForegroundColor Red
            $failures++
            $results += [PSCustomObject]@{ Name = $name; Status = 'TIMEOUT'; DiffPercent = 100.0; CaptureSec = [math]::Round($sw.Elapsed.TotalSeconds, 2); Capture = $shotPath; Diff = $null }
            continue
        }

        if (-not (Test-Path -LiteralPath $shotPath) -or (Get-Item -LiteralPath $shotPath).Length -eq 0) {
            Write-Host ("  [FAIL] {0,-20} Screenshot missing or empty (exit={1})" -f $name, $proc.ExitCode) -ForegroundColor Red
            $failures++
            $results += [PSCustomObject]@{ Name = $name; Status = 'MISSING'; DiffPercent = 100.0; CaptureSec = [math]::Round($sw.Elapsed.TotalSeconds, 2); Capture = $shotPath; Diff = $null }
            continue
        }

        if ($UpdateBaselines -or -not (Test-Path -LiteralPath $basePath)) {
            $w = 0; $h = 0
            $isBlank = [NhipVisualDiff]::CheckBlank($shotPath, [ref]$w, [ref]$h)
            if ($isBlank) {
                Write-Host ("  [FAIL] {0,-20} Captured frame is blank/solid ({1}x{2})" -f $name, $w, $h) -ForegroundColor Red
                $failures++
                $results += [PSCustomObject]@{ Name = $name; Status = 'BLANK'; Resolution = "${w}x${h}"; DiffPercent = 100.0; CaptureSec = [math]::Round($sw.Elapsed.TotalSeconds, 2); Capture = $shotPath; Diff = $null }
                continue
            }
            Copy-Item -LiteralPath $shotPath -Destination $basePath -Force
            $tag = if ($UpdateBaselines) { 'UPDATED' } else { 'BASELINE_CREATED' }
            Write-Host ("  [OK]   {0,-20} {1,-16} {2}x{3} ({4}s)" -f $name, $tag, $w, $h, [math]::Round($sw.Elapsed.TotalSeconds, 2)) -ForegroundColor Green
            $results += [PSCustomObject]@{ Name = $name; Status = $tag; Resolution = "${w}x${h}"; DiffPercent = 0.0; CaptureSec = [math]::Round($sw.Elapsed.TotalSeconds, 2); Capture = $shotPath; Baseline = $basePath; Diff = $null }
            continue
        }

        $cmp = [NhipVisualDiff]::Compare($shotPath, $basePath, $diffPath, $ChannelTolerance)
        $pct = [math]::Round($cmp.DiffPercent, 3)

        if ($cmp.IsBlank) {
            Write-Host ("  [FAIL] {0,-20} Blank/solid frame ({1}x{2})" -f $name, $cmp.Width, $cmp.Height) -ForegroundColor Red
            $failures++
            $status = 'BLANK'
        } elseif ($cmp.SizeMismatch) {
            Write-Host ("  [FAIL] {0,-20} Size mismatch: got {1}x{2}, baseline {3}x{4}" -f $name, $cmp.Width, $cmp.Height, $cmp.BaseWidth, $cmp.BaseHeight) -ForegroundColor Red
            $failures++
            $status = 'SIZE_MISMATCH'
        } elseif ($cmp.DiffPercent -gt $MaxDiffPercent) {
            Write-Host ("  [FAIL] {0,-20} Diff {1}% > {2}%  -> {3}" -f $name, $pct, $MaxDiffPercent, $diffPath) -ForegroundColor Red
            $failures++
            $status = 'VISUAL_DIFF'
        } else {
            if (Test-Path -LiteralPath $diffPath) { Remove-Item -LiteralPath $diffPath -Force }
            Write-Host ("  [PASS] {0,-20} diff={1}% ({2}x{3}, {4}s)" -f $name, $pct, $cmp.Width, $cmp.Height, [math]::Round($sw.Elapsed.TotalSeconds, 2)) -ForegroundColor Green
            $status = 'PASS'
        }

        $results += [PSCustomObject]@{
            Name        = $name
            Status      = $status
            Resolution  = "$($cmp.Width)x$($cmp.Height)"
            DiffPercent = $pct
            CaptureSec  = [math]::Round($sw.Elapsed.TotalSeconds, 2)
            Capture     = $shotPath
            Baseline    = $basePath
            Diff        = if ($status -eq 'VISUAL_DIFF') { $diffPath } else { $null }
        }
    }

    $report = [PSCustomObject]@{
        Timestamp        = (Get-Date).ToString('o')
        Mode             = if ($Quick) { 'quick' } else { 'full' }
        MaxDiffPercent   = $MaxDiffPercent
        ChannelTolerance = $ChannelTolerance
        Failures         = $failures
        Results          = $results
    }
    $report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $reportPath -Encoding UTF8

    Write-Host "`nReport saved to: $reportPath" -ForegroundColor DarkGray
    if ($failures -gt 0) {
        Write-Host "UX/UI suite finished with $failures failure(s)." -ForegroundColor Red
        exit 1
    }
    Write-Host "UX/UI suite PASSED ($($results.Count)/$($results.Count) screens)." -ForegroundColor Green
    exit 0
} finally {
    Pop-Location
}
