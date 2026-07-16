import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'

import { openJobEventStream } from '@/api/client'
import type { JobDetail } from '@/api/types'

const terminalStatuses = new Set(['succeeded', 'cancelled', 'dead_lettered', 'expired'])

export function useJobEvents(jobId: string | undefined, enabled: boolean): void {
  const queryClient = useQueryClient()

  useEffect(() => {
    if (!jobId || !enabled) return
    const controller = new AbortController()
    let retryTimer: ReturnType<typeof setTimeout> | undefined
    let retries = 0

    const reconnect = (): void => {
      if (controller.signal.aborted) return
      retries += 1
      retryTimer = setTimeout(() => void connect(), Math.min(1000 * 2 ** retries, 10_000))
    }

    const connect = async (): Promise<void> => {
      try {
        await openJobEventStream(jobId, controller.signal, (detail: JobDetail) => {
          retries = 0
          queryClient.setQueryData(['job', jobId], detail)
          void queryClient.invalidateQueries({ queryKey: ['jobs'] })
          if (terminalStatuses.has(detail.status)) controller.abort()
        })
        reconnect()
      } catch {
        reconnect()
      }
    }

    void connect()
    return () => {
      controller.abort()
      if (retryTimer) clearTimeout(retryTimer)
    }
  }, [enabled, jobId, queryClient])
}
