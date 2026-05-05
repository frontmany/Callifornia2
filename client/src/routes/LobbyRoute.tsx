import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import meeting from '@/assets/icons/meeting.png'
import { LinkShareDialog } from '@/components/LinkShareDialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { callOrchestrator } from '@/services/callOrchestrator'
import { useCallStore } from '@/stores/callStore'

export function LobbyRoute() {
  const [roomId, setRoomId] = useState('')
  const navigate = useNavigate()
  const currentRoomId = useCallStore((s) => s.roomId)

  const create = async () => {
    await callOrchestrator.connectAndCreate()
    const room = useCallStore.getState().roomId
    if (room) navigate(`/r/${room}`)
  }

  const join = async () => {
    await callOrchestrator.connectAndJoin(roomId)
    navigate(`/r/${roomId}`)
  }

  return (
    <div className="mx-auto max-w-xl p-6">
      <div className="mb-4 flex items-center gap-3">
        <img src={meeting} alt="meeting" className="h-8 w-8" />
        <h2 className="text-xl font-semibold">Lobby</h2>
      </div>
      <div className="space-y-3 rounded-xl border border-border bg-card p-4">
        <Button className="w-full" onClick={create}>Create room</Button>
        <Input value={roomId} onChange={(e) => setRoomId(e.target.value)} placeholder="Room UUID" />
        <Button className="w-full bg-secondary" onClick={join}>Join room</Button>
        {currentRoomId ? <LinkShareDialog roomId={currentRoomId} /> : null}
      </div>
    </div>
  )
}
