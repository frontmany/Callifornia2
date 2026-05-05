import { useEffect } from 'react'

export function useReconnect(callback: () => void, delayMs = 5000) {
  useEffect(() => {
    const id = window.setTimeout(callback, delayMs)
    return () => window.clearTimeout(id)
  }, [callback, delayMs])
}
