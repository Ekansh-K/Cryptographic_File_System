import React from 'react'
import { Button, Tooltip } from 'antd'
import { SunOutlined, MoonOutlined, DesktopOutlined } from '@ant-design/icons'
import { motion } from 'framer-motion'
import { useTheme } from '../contexts/ThemeContext'

interface ThemeToggleProps {
  className?: string
  size?: 'small' | 'middle' | 'large'
  showLabel?: boolean
}

const ThemeToggle: React.FC<ThemeToggleProps> = ({ 
  className = '', 
  size = 'middle',
  showLabel = false 
}) => {
  const { theme, setTheme, isDark } = useTheme()

  const getNextTheme = () => {
    switch (theme) {
      case 'light':
        return 'dark'
      case 'dark':
        return 'system'
      case 'system':
        return 'light'
      default:
        return 'light'
    }
  }

  const getCurrentIcon = () => {
    switch (theme) {
      case 'light':
        return <SunOutlined />
      case 'dark':
        return <MoonOutlined />
      case 'system':
        return <DesktopOutlined />
      default:
        return <SunOutlined />
    }
  }

  const getCurrentLabel = () => {
    switch (theme) {
      case 'light':
        return 'Light'
      case 'dark':
        return 'Dark'
      case 'system':
        return 'System'
      default:
        return 'Light'
    }
  }

  const getTooltipText = () => {
    const nextTheme = getNextTheme()
    return `Switch to ${nextTheme} theme`
  }

  const handleToggle = () => {
    const nextTheme = getNextTheme()
    setTheme(nextTheme)
  }

  return (
    <Tooltip title={getTooltipText()} placement="bottom">
      <motion.div
        whileHover={{ scale: 1.05 }}
        whileTap={{ scale: 0.95 }}
        className={className}
      >
        <Button
          type="text"
          size={size}
          icon={getCurrentIcon()}
          onClick={handleToggle}
          className={`
            glass-button
            flex items-center justify-center
            transition-all duration-300 ease-in-out
            hover:shadow-lg
            ${isDark ? 'text-slate-200 hover:text-white' : 'text-slate-700 hover:text-slate-900'}
            ${showLabel ? 'px-4' : ''}
          `}
          style={{
            background: 'var(--glass-bg)',
            backdropFilter: 'var(--glass-backdrop)',
            border: '1px solid var(--glass-border)',
            borderRadius: '12px',
          }}
        >
          {showLabel && (
            <span className="ml-2 font-medium">
              {getCurrentLabel()}
            </span>
          )}
        </Button>
      </motion.div>
    </Tooltip>
  )
}

export default ThemeToggle