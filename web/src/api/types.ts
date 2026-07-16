import type { components } from '@/api/schema'

export type Job = components['schemas']['Job']
export type JobAttempt = components['schemas']['JobAttempt']
export type JobDetail = components['schemas']['JobDetail']
export type JobKind = components['schemas']['JobKind']
export type JobOutput = components['schemas']['JobOutput']
export type JobPage = components['schemas']['JobPage']
export type JobStatus = components['schemas']['JobStatus']
export type ProblemDetails = components['schemas']['ProblemDetails']
export type SessionResponse = components['schemas']['SessionResponse']

export interface JobFilters {
  status?: JobStatus | 'all'
  kind?: JobKind | 'all'
  cursor?: string
}
