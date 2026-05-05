import camera from '@/assets/icons/camera.png'
import microphone from '@/assets/icons/microphone.png'
import screen from '@/assets/icons/screenShare.png'
import decline from '@/assets/icons/declineCall.png'
import { Button } from '@/components/ui/button'

type Props = {
  micEnabled: boolean
  camEnabled: boolean
  screenEnabled: boolean
  onMicToggle: () => void
  onCamToggle: () => void
  onScreenToggle: () => void
  onLeave: () => void
}

function IconButton({ icon, label, onClick }: { icon: string; label: string; onClick: () => void }) {
  return (
    <Button onClick={onClick} className="h-12 w-12 rounded-full bg-secondary p-0">
      <img src={icon} alt={label} className="h-5 w-5" />
    </Button>
  )
}

export function ControlBar({ onMicToggle, onCamToggle, onScreenToggle, onLeave }: Props) {
  return (
    <div className="fixed bottom-6 left-1/2 flex -translate-x-1/2 gap-3 rounded-full border border-border bg-card p-3">
      <IconButton icon={microphone} label="Microphone" onClick={onMicToggle} />
      <IconButton icon={camera} label="Camera" onClick={onCamToggle} />
      <IconButton icon={screen} label="Screen" onClick={onScreenToggle} />
      <IconButton icon={decline} label="Leave" onClick={onLeave} />
    </div>
  )
}
