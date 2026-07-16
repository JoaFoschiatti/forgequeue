[CmdletBinding()]
param(
    [string]$ApiUrl = 'http://localhost:8080',
    [int]$TimeoutSeconds = 120
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

function Invoke-CurlJson {
    param([string[]]$CurlArguments)

    $body = & curl.exe @CurlArguments
    if ($LASTEXITCODE -ne 0) {
        throw "curl terminó con código $LASTEXITCODE"
    }
    return $body | ConvertFrom-Json
}

function Get-Job {
    param([string]$JobId, [string]$Token)

    return Invoke-CurlJson @(
        '-fsS',
        '-H', "Authorization: Bearer $Token",
        "$ApiUrl/api/v1/jobs/$JobId"
    )
}

Write-Host '→ Preparando servicios base...' -ForegroundColor Cyan
& docker compose up --build -d postgres minio minio-init api web
if ($LASTEXITCODE -ne 0) { throw 'docker compose up falló' }
& docker compose stop worker-1 worker-2 2>$null | Out-Null
& docker rm -f forgequeue-chaos 2>$null | Out-Null

$deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
do {
    & curl.exe -fsS "$ApiUrl/health/ready" 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) { break }
    Start-Sleep -Seconds 1
} while ([DateTime]::UtcNow -lt $deadline)
if ($LASTEXITCODE -ne 0) { throw 'La API no quedó lista dentro del plazo' }

$fixture = Join-Path ([IO.Path]::GetTempPath()) "forgequeue-chaos-$([Guid]::NewGuid()).png"
$png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII='
[IO.File]::WriteAllBytes($fixture, [Convert]::FromBase64String($png))

try {
    Write-Host '→ Iniciando worker de caos (lease 10 s, pausa 45 s)...' -ForegroundColor Cyan
    $chaosContainer = (& docker compose run -d --no-deps --name forgequeue-chaos `
        -e WORKER_ID=chaos-worker `
        -e LEASE_SECONDS=10 `
        -e HEARTBEAT_SECONDS=2 `
        -e DEMO_PROCESSING_DELAY_MILLISECONDS=45000 `
        worker-1 worker).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $chaosContainer) { throw 'No se pudo iniciar el worker de caos' }

    $session = Invoke-CurlJson @('-fsS', '-X', 'POST', "$ApiUrl/api/v1/sessions")
    $job = Invoke-CurlJson @(
        '-sS',
        '-H', "Authorization: Bearer $($session.token)",
        '-H', "Idempotency-Key: chaos-$([Guid]::NewGuid())",
        '-F', "file=@$fixture;type=image/png",
        "$ApiUrl/api/v1/jobs"
    )
    if (-not $job.id) { throw "La carga falló: $($job.detail)" }
    Write-Host "  Trabajo: $($job.id)" -ForegroundColor DarkGray

    do {
        $detail = Get-Job -JobId $job.id -Token $session.token
        Write-Host "  estado=$($detail.status) etapa=$($detail.stage) intento=$($detail.attempt_count)"
        if ($detail.status -eq 'running' -and $detail.stage -eq 'demo_delay') { break }
        if ($detail.status -in @('dead_lettered', 'cancelled', 'expired')) {
            throw "El trabajo terminó antes de la caída controlada: $($detail.status)"
        }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($detail.stage -ne 'demo_delay') { throw 'El worker no tomó el trabajo dentro del plazo' }

    Write-Host '→ Matando el worker con un lease activo...' -ForegroundColor Yellow
    & docker stop forgequeue-chaos | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'No se pudo detener el worker de caos' }

    Write-Host '→ Iniciando worker de recuperación...' -ForegroundColor Cyan
    & docker compose up -d worker-2 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'No se pudo iniciar worker-2' }

    do {
        $detail = Get-Job -JobId $job.id -Token $session.token
        Write-Host "  estado=$($detail.status) etapa=$($detail.stage) intento=$($detail.attempt_count)"
        if ($detail.status -eq 'succeeded') { break }
        if ($detail.status -in @('dead_lettered', 'cancelled', 'expired')) {
            throw "La recuperación terminó en $($detail.status)"
        }
        Start-Sleep -Seconds 1
    } while ([DateTime]::UtcNow -lt $deadline)

    if ($detail.status -ne 'succeeded') { throw 'La recuperación excedió el plazo' }
    $workerLost = @($detail.attempts | Where-Object { $_.error_code -eq 'worker_lost' })
    if ($detail.attempt_count -lt 2 -or $workerLost.Count -ne 1) {
        throw 'El trabajo terminó, pero no conserva la evidencia esperada de recuperación'
    }

    Write-Host "✓ Recuperado: $($detail.attempt_count) intentos, $($detail.outputs.Count) artefactos." -ForegroundColor Green
    Write-Host "  Abrir: http://localhost:5173/jobs/$($job.id)" -ForegroundColor Green
}
finally {
    Remove-Item -LiteralPath $fixture -Force -ErrorAction SilentlyContinue
    & docker rm -f forgequeue-chaos 2>$null | Out-Null
}
