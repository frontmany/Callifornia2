import { useMemo } from 'react'

type Props = {
  nickname: string
  stream?: MediaStream
}

export function ParticipantTile({ nickname, stream }: Props) {
  const url = useMemo(() => (stream ? URL.createObjectURL(stream as unknown as Blob) : null), [stream])

  return (
    <div className="rounded-xl border border-border bg-card p-3">
      {stream ? (
        <video autoPlay playsInline muted={false} className="aspect-video w-full rounded-md bg-black" />
      ) : (
        <div className="aspect-video w-full rounded-md bg-muted" />
      )}
      <p className="mt-2 text-sm text-muted-foreground">{nickname}</p>
      {url ? <span className="hidden">{url}</span> : null}
    </div>
  )
}
