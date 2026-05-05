import { forwardRef, type ButtonHTMLAttributes } from 'react'
import { cn } from '@/lib/utils'

export const Button = forwardRef<HTMLButtonElement, ButtonHTMLAttributes<HTMLButtonElement>>(
  ({ className, ...props }, ref) => (
    <button
      ref={ref}
      className={cn(
        'inline-flex items-center justify-center rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50',
        className
      )}
      {...props}
    />
  )
)

Button.displayName = 'Button'
