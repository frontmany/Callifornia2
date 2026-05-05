import copy from '@/assets/icons/copy.png'
import copied from '@/assets/icons/copied.png'
import { useState } from 'react'

export function LinkShareDialog({ roomId }: { roomId: string }) {
  const [done, setDone] = useState(false)
  const link = `${window.location.origin}/r/${roomId}`

  async function onCopy() {
    await navigator.clipboard.writeText(link)
    setDone(true)
    window.setTimeout(() => setDone(false), 1000)
  }

  return (
    <button
      onClick={onCopy}
      className="inline-flex items-center gap-2 rounded-md border border-border bg-card px-3 py-2 text-sm"
    >
      <img src={done ? copied : copy} alt="copy" className="h-4 w-4" />
      Copy invite link
    </button>
  )
}
