import { render, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { describe, expect, it } from 'vitest'

import type { Job } from '@/api/types'
import { JobTable } from '@/components/job-table'

const job: Job = {
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
}

describe('JobTable', () => {
  it('presenta estado, tipo y enlace de detalle', () => {
    render(<MemoryRouter><JobTable jobs={[job]} /></MemoryRouter>)
    expect(screen.getByText('portfolio.png')).toBeInTheDocument()
    expect(screen.getByText('Completado')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'portfolio.png' })).toHaveAttribute('href', `/jobs/${job.id}`)
  })

  it('diseña explícitamente el estado vacío', () => {
    render(<MemoryRouter><JobTable jobs={[]} /></MemoryRouter>)
    expect(screen.getByText('Todavía no hay trabajos')).toBeInTheDocument()
  })
})
