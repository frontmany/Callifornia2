import { useEffect } from 'react'
import { useParams } from 'react-router-dom'
import { ControlBar } from '@/components/ControlBar'
import { DeviceMenu } from '@/components/DeviceMenu'
import { ErrorBoundary } from '@/components/ErrorBoundary'
import { VideoGrid } from '@/components/VideoGrid'
import { callOrchestrator } from '@/services/callOrchestrator'
import { useCallStore } from '@/stores/callStore'
import { useMediaStore } from '@/stores/mediaStore'

export function RoomRoute() {
  const { roomId } = useParams()
  const participants = useCallStore((s) => s.participants)
  const error = useCallStore((s) => s.lastError)
  const streams = useMediaStore((s) => s.remoteStreams)
  const micEnabled = useMediaStore((s) => s.micEnabled)
  const camEnabled = useMediaStore((s) => s.camEnabled)
  const screenEnabled = useMediaStore((s) => s.screenEnabled)

  useEffect(() => {
    if (roomId) void callOrchestrator.connectAndJoin(roomId)
    return () => callOrchestrator.leave()
  }, [roomId])

  if (error) return <ErrorBoundary message={error} />

  return (
    <div className="p-6 pb-28">
      <div className="mb-4 flex items-center justify-between">
        <h2 className="text-lg font-semibold">Room: {roomId}</h2>
        <DeviceMenu />
      </div>
      <VideoGrid participants={participants} streams={streams} />
      <ControlBar
        micEnabled={micEnabled}
        camEnabled={camEnabled}
        screenEnabled={screenEnabled}
        onMicToggle={() => void callOrchestrator.toggleMic(!micEnabled)}
        onCamToggle={() => void callOrchestrator.toggleCam(!camEnabled)}
        onScreenToggle={() => void callOrchestrator.toggleScreen(!screenEnabled)}
        onLeave={() => callOrchestrator.leave()}
      />
    </div>
  )
}
