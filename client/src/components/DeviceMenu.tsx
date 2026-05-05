import settings from '@/assets/icons/settings.png'

export function DeviceMenu() {
  return (
    <button className="inline-flex items-center gap-2 rounded-md border border-border bg-card px-3 py-2 text-sm">
      <img src={settings} alt="settings" className="h-4 w-4" />
      Devices
    </button>
  )
}
