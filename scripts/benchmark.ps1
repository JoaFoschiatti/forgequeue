[CmdletBinding()]
param(
    [ValidateRange(1, 1000)]
    [int]$Count = 100,
    [string]$ApiUrl = 'http://localhost:8080',
    [string[]]$MetricsUrls = @('http://localhost:8080/metrics', 'http://localhost:9101/metrics', 'http://localhost:9102/metrics'),
    [int]$TimeoutSeconds = 180
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

function Invoke-CurlJson {
    param([string[]]$CurlArguments)

    $body = & curl.exe @CurlArguments
    if ($LASTEXITCODE -ne 0) { throw "curl terminó con código $LASTEXITCODE" }
    return $body | ConvertFrom-Json
}

function Add-ImageJob {
    param([string]$Token, [string]$Fixture)

    $result = Invoke-CurlJson @(
        '-sS',
        '-H', "Authorization: Bearer $Token",
        '-H', "Idempotency-Key: benchmark-$([Guid]::NewGuid())",
        '-F', "file=@$Fixture;type=image/png",
        "$ApiUrl/api/v1/jobs"
    )
    if (-not $result.id) {
        throw "La API rechazó el benchmark: $($result.code) $($result.detail). Elevá las cuotas como indica docs/BENCHMARKS.md."
    }
    return $result
}

function Get-SessionJobs {
    param([string]$Token)

    $items = [Collections.Generic.List[object]]::new()
    $cursor = $null
    do {
        $url = "$ApiUrl/api/v1/jobs"
        if ($cursor) { $url += "?cursor=$cursor" }
        $page = Invoke-CurlJson @('-fsS', '-H', "Authorization: Bearer $Token", $url)
        foreach ($item in $page.items) { $items.Add($item) }
        $cursor = $page.next_cursor
    } while ($cursor)
    return $items
}

function Get-Percentile {
    param([double[]]$Values, [double]$Percentile)

    $sorted = @($Values | Sort-Object)
    $index = [Math]::Max(0, [Math]::Ceiling($Percentile * $sorted.Count) - 1)
    return [Math]::Round($sorted[$index], 2)
}

& curl.exe -fsS "$ApiUrl/health/ready" | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'La API no está lista' }

$fixture = Join-Path ([IO.Path]::GetTempPath()) "forgequeue-benchmark-$([Guid]::NewGuid()).png"
$png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII='
[IO.File]::WriteAllBytes($fixture, [Convert]::FromBase64String($png))

try {
    $session = Invoke-CurlJson @('-fsS', '-X', 'POST', "$ApiUrl/api/v1/sessions")
    Write-Host "→ Encolando $Count imágenes..." -ForegroundColor Cyan
    $ids = [Collections.Generic.List[string]]::new()
    $latencies = [Collections.Generic.List[double]]::new()
    $total = [Diagnostics.Stopwatch]::StartNew()

    for ($index = 1; $index -le $Count; $index++) {
        $request = [Diagnostics.Stopwatch]::StartNew()
        $job = Add-ImageJob -Token $session.token -Fixture $fixture
        $request.Stop()
        $ids.Add([string]$job.id)
        $latencies.Add($request.Elapsed.TotalMilliseconds)
        if ($index % 10 -eq 0 -or $index -eq $Count) {
            Write-Host "  aceptadas: $index/$Count"
        }
    }

    $terminal = @('succeeded', 'dead_lettered', 'cancelled', 'expired')
    $targetIds = [Collections.Generic.HashSet[string]]::new([string[]]$ids)
    $details = @{}
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ($details.Count -lt $Count -and [DateTime]::UtcNow -lt $deadline) {
        foreach ($detail in @(Get-SessionJobs -Token $session.token)) {
            if ($targetIds.Contains([string]$detail.id) -and $detail.status -in $terminal) {
                $details[[string]$detail.id] = $detail
            }
        }
        Write-Host "  terminados: $($details.Count)/$Count" -NoNewline
        Write-Host "`r" -NoNewline
        if ($details.Count -lt $Count) { Start-Sleep -Milliseconds 500 }
    }
    $total.Stop()
    Write-Host

    if ($details.Count -lt $Count) { throw "$($Count - $details.Count) trabajos excedieron el plazo" }
    $succeeded = @($details.Values | Where-Object { $_.status -eq 'succeeded' }).Count
    $failed = $Count - $succeeded
    $attempts = ($details.Values | Measure-Object -Property attempt_count -Sum).Sum
    $throughput = [Math]::Round($Count / $total.Elapsed.TotalSeconds, 2)

    Write-Host '✓ Benchmark completo' -ForegroundColor Green
    Write-Host "  total:       $([Math]::Round($total.Elapsed.TotalSeconds, 2)) s"
    Write-Host "  throughput:  $throughput trabajos/s"
    Write-Host "  aceptación:  p50=$(Get-Percentile $latencies 0.50) ms · p95=$(Get-Percentile $latencies 0.95) ms"
    Write-Host "  resultados:  $succeeded exitosos · $failed fallidos · $attempts intentos"
    Write-Host
    Write-Host 'Métricas Prometheus:' -ForegroundColor DarkGray
    foreach ($metricsUrl in @($MetricsUrls | ForEach-Object { $_ -split ',' })) {
        Write-Host "  $metricsUrl" -ForegroundColor DarkGray
        $metricsOutput = & curl.exe -fsS $metricsUrl 2>$null
        if ($LASTEXITCODE -eq 0) {
            $metricsOutput | Select-String 'forgequeue_(jobs|job_duration|leases)'
        } else {
            Write-Host '    no disponible' -ForegroundColor Yellow
        }
    }
}
finally {
    Remove-Item -LiteralPath $fixture -Force -ErrorAction SilentlyContinue
}
