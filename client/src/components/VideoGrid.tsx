import { ParticipantTile } from './ParticipantTile'

type Props = {
  participants: string[]
  streams: Record<string, MediaStream>
}

export function VideoGrid({ participants, streams }: Props) {
  return (
    <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
      {participants.map((participant) => (
        <ParticipantTile key={participant} nickname={participant} stream={streams[participant]} />
      ))}
    </div>
  )
}
