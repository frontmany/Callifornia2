import { AlertTriangle } from 'lucide-react'

export function ErrorBoundary({ message }: { message: string }) {
  return (
    <div className="mx-auto mt-10 max-w-md rounded-lg border border-destructive/30 bg-destructive/10 p-6 text-center">
      <AlertTriangle className="mx-auto mb-3 h-8 w-8 text-destructive" />
      <p className="text-sm text-destructive-foreground">{message}</p>
    </div>
  )
}
