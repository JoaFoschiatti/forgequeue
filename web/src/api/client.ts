import type {
  Job,
  JobDetail,
  JobFilters,
  JobPage,
  ProblemDetails,
  SessionResponse,
} from '@/api/types'

const API_URL = (import.meta.env.VITE_API_URL ?? 'http://localhost:8080').replace(/\/$/, '')
const SESSION_STORAGE_KEY = 'forgequeue.session-token'

let pendingSession: Promise<string> | undefined

export class ApiError extends Error {
  readonly status: number
  readonly problem?: ProblemDetails

  constructor(status: number, message: string, problem?: ProblemDetails) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.problem = problem
  }
}

export function apiUrl(path: string): string {
  return `${API_URL}${path}`
}

export function clearSession(): void {
  localStorage.removeItem(SESSION_STORAGE_KEY)
  pendingSession = undefined
}

export async function ensureSession(): Promise<string> {
  const existing = localStorage.getItem(SESSION_STORAGE_KEY)
  if (existing) return existing
  if (pendingSession) return pendingSession

  pendingSession = fetch(apiUrl('/api/v1/sessions'), { method: 'POST' })
    .then(async (response) => {
      if (!response.ok) throw await toApiError(response)
      const session = (await response.json()) as SessionResponse
      localStorage.setItem(SESSION_STORAGE_KEY, session.token)
      return session.token
    })
    .finally(() => {
      pendingSession = undefined
    })

  return pendingSession
}

export async function health(signal?: AbortSignal): Promise<boolean> {
  const response = await fetch(apiUrl('/health/ready'), { signal })
  if (!response.ok) throw new Error(`Backend no disponible (${response.status})`)
  return true
}

export async function uploadJob(file: File): Promise<Job> {
  const token = await ensureSession()
  const form = new FormData()
  form.append('file', file)
  const response = await fetch(apiUrl('/api/v1/jobs'), {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${token}`,
      'Idempotency-Key': crypto.randomUUID(),
    },
    body: form,
  })
  if (!response.ok) throw await toApiError(response)
  return response.json() as Promise<Job>
}

export async function listJobs(filters: JobFilters = {}): Promise<JobPage> {
  const query = new URLSearchParams()
  if (filters.status && filters.status !== 'all') query.set('status', filters.status)
  if (filters.kind && filters.kind !== 'all') query.set('kind', filters.kind)
  if (filters.cursor) query.set('cursor', filters.cursor)
  return authorizedJson<JobPage>(`/api/v1/jobs?${query.toString()}`)
}

export async function getJob(jobId: string): Promise<JobDetail> {
  return authorizedJson<JobDetail>(`/api/v1/jobs/${jobId}`)
}

export async function cancelJob(jobId: string): Promise<Job> {
  return authorizedJson<Job>(`/api/v1/jobs/${jobId}/cancel`, { method: 'POST' })
}

export async function retryJob(jobId: string): Promise<Job> {
  return authorizedJson<Job>(`/api/v1/jobs/${jobId}/retry`, { method: 'POST' })
}

export async function deleteJob(jobId: string): Promise<void> {
  await authorized(`/api/v1/jobs/${jobId}`, { method: 'DELETE' })
}

export async function downloadOutput(
  jobId: string,
  outputId: string,
  signal?: AbortSignal,
): Promise<Blob> {
  const response = await authorized(`/api/v1/jobs/${jobId}/outputs/${outputId}`, { signal })
  return response.blob()
}

export async function openJobEventStream(
  jobId: string,
  signal: AbortSignal,
  onDetail: (detail: JobDetail) => void,
): Promise<void> {
  const response = await authorized(`/api/v1/jobs/${jobId}/events`, {
    headers: { Accept: 'text/event-stream' },
    signal,
  })
  if (!response.body) throw new Error('El navegador no recibió el flujo de eventos.')

  const reader = response.body.pipeThrough(new TextDecoderStream()).getReader()
  let buffer = ''
  while (!signal.aborted) {
    const { done, value } = await reader.read()
    if (done) break
    buffer += value
    buffer = buffer.replaceAll('\r\n', '\n')
    let boundary = buffer.indexOf('\n\n')
    while (boundary >= 0) {
      const block = buffer.slice(0, boundary)
      buffer = buffer.slice(boundary + 2)
      const data = block
        .split('\n')
        .filter((line) => line.startsWith('data:'))
        .map((line) => line.slice(5).trimStart())
        .join('\n')
      if (data) onDetail(JSON.parse(data) as JobDetail)
      boundary = buffer.indexOf('\n\n')
    }
  }
}

async function authorizedJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await authorized(path, init)
  return response.json() as Promise<T>
}

async function authorized(path: string, init: RequestInit = {}): Promise<Response> {
  const token = await ensureSession()
  const headers = new Headers(init.headers)
  headers.set('Authorization', `Bearer ${token}`)
  const response = await fetch(apiUrl(path), { ...init, headers })
  if (!response.ok) {
    const error = await toApiError(response)
    if (response.status === 401) clearSession()
    throw error
  }
  return response
}

async function toApiError(response: Response): Promise<ApiError> {
  let problem: ProblemDetails | undefined
  try {
    problem = (await response.json()) as ProblemDetails
  } catch {
    // A sleeping proxy can return HTML instead of the API error contract.
  }
  return new ApiError(
    response.status,
    problem?.detail ?? `La API respondió con estado ${response.status}.`,
    problem,
  )
}
