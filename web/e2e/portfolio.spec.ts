import { expect, test } from '@playwright/test'

const job = {
  id: '01900000-0000-7000-8000-000000000001',
  kind: 'image',
  status: 'succeeded',
  progress: 100,
  stage: 'completed',
  original_name: 'portfolio.png',
  input_content_type: 'image/png',
  input_size: 2048,
  attempt_count: 1,
  max_attempts: 3,
  created_at: '2026-07-16T12:00:00Z',
  updated_at: '2026-07-16T12:00:02Z',
  completed_at: '2026-07-16T12:00:02Z',
}

test.beforeEach(async ({ page }) => {
  await page.route('**/health/ready', (route) => route.fulfill({ status: 200, body: 'ok' }))
  await page.route('**/api/v1/sessions', (route) => route.fulfill({
    status: 201,
    contentType: 'application/json',
    body: JSON.stringify({ token: 'fq_test', expires_at: '2026-07-17T12:00:00Z' }),
  }))
  await page.route('**/api/v1/jobs?*', (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ items: [job], next_cursor: null }),
  }))
  await page.route(`**/api/v1/jobs/${job.id}`, (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ ...job, attempts: [], outputs: [] }),
  }))
})

test('la portada explica y ofrece la carga principal', async ({ page }) => {
  await page.goto('/')
  await expect(page.getByRole('heading', { name: /Trabajo pesado fuera/ })).toBeVisible()
  await expect(page.getByText('Elegir archivo', { exact: true })).toBeVisible()
  await expect(page.getByText('Servicio listo')).toBeVisible()
})

test('el historial abre el detalle del trabajo', async ({ page }) => {
  await page.goto('/jobs')
  await expect(page.getByText('portfolio.png')).toBeVisible()
  await page.getByRole('link', { name: 'portfolio.png', exact: true }).click()
  await expect(page.getByRole('heading', { name: 'portfolio.png' })).toBeVisible()
  await expect(page.getByText('Procesamiento completado')).toBeVisible()
})

test('la carga encola el archivo y navega al seguimiento', async ({ page }) => {
  await page.route('**/api/v1/jobs', async (route) => {
    expect(route.request().method()).toBe('POST')
    expect(route.request().headers()['idempotency-key']).toBeTruthy()
    await route.fulfill({
      status: 202,
      contentType: 'application/json',
      body: JSON.stringify({ ...job, status: 'queued', progress: 0, stage: 'queued' }),
    })
  })

  await page.goto('/')
  await page.getByLabel('Elegir archivo').setInputFiles({
    name: 'portfolio.png',
    mimeType: 'image/png',
    buffer: Buffer.from('fixture'),
  })
  await page.getByRole('button', { name: 'Comenzar procesamiento' }).click()
  await expect(page).toHaveURL(`/jobs/${job.id}`)
  await expect(page.getByRole('heading', { name: 'portfolio.png' })).toBeVisible()
})

test('los filtros actualizan la consulta del historial', async ({ page }) => {
  const requests: string[] = []
  await page.route('**/api/v1/jobs?*', async (route) => {
    requests.push(route.request().url())
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ items: [], next_cursor: null }),
    })
  })

  await page.goto('/jobs')
  await page.getByRole('combobox', { name: 'Filtrar por tipo' }).click()
  await page.getByRole('option', { name: 'PDFs' }).click()
  await page.getByRole('combobox', { name: 'Filtrar por estado' }).click()
  await page.getByRole('option', { name: 'Fallidos' }).click()

  await expect.poll(() => requests.some((url) => url.includes('kind=pdf') && url.includes('status=dead_lettered'))).toBe(true)
})

test('SSE actualiza el progreso hasta el estado terminal', async ({ page }) => {
  const running = { ...job, status: 'running', progress: 25, stage: 'decoding_image' }
  const completed = { ...job, status: 'succeeded', progress: 100, stage: 'completed' }
  await page.route(`**/api/v1/jobs/${job.id}`, (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ ...running, attempts: [], outputs: [] }),
  }))
  await page.route(`**/api/v1/jobs/${job.id}/events`, (route) => route.fulfill({
    status: 200,
    contentType: 'text/event-stream',
    body: `event: job.updated\ndata: ${JSON.stringify({ ...completed, attempts: [], outputs: [] })}\n\n`,
  }))

  await page.goto(`/jobs/${job.id}`)
  await expect(page.getByText('Procesamiento completado')).toBeVisible()
  await expect(page.getByLabel('Progreso 100%')).toBeVisible()
})

test('muestra el fallo definitivo y permite descargar un artefacto privado', async ({ page }) => {
  const output = {
    id: '01900000-0000-7000-8000-000000000099',
    name: 'page-1.png',
    content_type: 'image/png',
    size: 68,
    width: 1,
    height: 1,
    page_number: 1,
    created_at: '2026-07-16T12:00:02Z',
  }
  await page.route(`**/api/v1/jobs/${job.id}`, (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({
      ...job,
      status: 'dead_lettered',
      progress: 42,
      stage: 'dead_lettered',
      last_error_code: 'invalid_input',
      last_error_detail: 'El documento está corrupto.',
      attempts: [],
      outputs: [output],
    }),
  }))
  await page.route(`**/api/v1/jobs/${job.id}/outputs/${output.id}`, (route) => route.fulfill({
    status: 200,
    contentType: 'image/png',
    body: Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=', 'base64'),
  }))

  await page.goto(`/jobs/${job.id}`)
  await expect(page.getByText('Fallido definitivamente')).toBeVisible()
  await expect(page.getByText('El documento está corrupto.')).toBeVisible()
  await expect(page.getByRole('img', { name: 'Resultado page-1.png' })).toBeVisible()
  await expect(page.getByRole('link', { name: 'Descargar' })).toHaveAttribute('download', 'page-1.png')
})

test('permite solicitar la cancelación de un trabajo activo', async ({ page }) => {
  const running = { ...job, status: 'running', progress: 25, stage: 'decoding_image' }
  let cancelRequested = false
  await page.route(`**/api/v1/jobs/${job.id}`, (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ ...running, attempts: [], outputs: [] }),
  }))
  await page.route(`**/api/v1/jobs/${job.id}/events`, (route) => route.abort())
  await page.route(`**/api/v1/jobs/${job.id}/cancel`, (route) => {
    cancelRequested = route.request().method() === 'POST'
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ ...running, status: 'cancel_requested', stage: 'cancel_requested' }),
    })
  })

  await page.goto(`/jobs/${job.id}`)
  await page.getByRole('button', { name: 'Cancelar' }).click()
  await expect.poll(() => cancelRequested).toBe(true)
  await expect(page.getByText('Cancelación solicitada')).toBeVisible()
})

test('crea un trabajo nuevo al reintentar un fallo definitivo', async ({ page }) => {
  const retryId = '01900000-0000-7000-8000-000000000002'
  const failed = {
    ...job,
    status: 'dead_lettered',
    progress: 50,
    stage: 'dead_lettered',
    last_error_code: 'processing_failed',
    last_error_detail: 'Fallo transitorio agotado.',
  }
  await page.route(`**/api/v1/jobs/${job.id}`, (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ ...failed, attempts: [], outputs: [] }),
  }))
  await page.route(`**/api/v1/jobs/${job.id}/retry`, (route) => route.fulfill({
    status: 202,
    contentType: 'application/json',
    body: JSON.stringify({
      ...job,
      id: retryId,
      status: 'queued',
      progress: 0,
      stage: 'queued',
      retry_of_job_id: job.id,
    }),
  }))

  await page.goto(`/jobs/${job.id}`)
  await page.getByRole('button', { name: 'Volver a intentar' }).click()
  await expect(page).toHaveURL(`/jobs/${retryId}`)
})

test('confirma y elimina un trabajo terminal', async ({ page }) => {
  let deleted = false
  await page.route(`**/api/v1/jobs/${job.id}`, (route) => {
    if (route.request().method() === 'DELETE') {
      deleted = true
      return route.fulfill({ status: 204 })
    }
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ ...job, attempts: [], outputs: [] }),
    })
  })

  await page.goto(`/jobs/${job.id}`)
  await page.getByRole('button', { name: 'Eliminar' }).click()
  const dialog = page.getByRole('alertdialog')
  await expect(dialog).toBeVisible()
  await dialog.getByRole('button', { name: 'Eliminar' }).click()
  await expect.poll(() => deleted).toBe(true)
  await expect(page).toHaveURL('/jobs')
})
