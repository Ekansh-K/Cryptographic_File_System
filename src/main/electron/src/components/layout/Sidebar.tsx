import { useState, useEffect } from 'react'
import { Layout, Menu, Tooltip, Badge } from 'antd'
import { useNavigate, useLocation } from 'react-router-dom'
import {
  DashboardOutlined,
  FolderOutlined,
  SettingOutlined,
  LockOutlined,
  MenuFoldOutlined,
  MenuUnfoldOutlined,
  InfoCircleOutlined,
  ShareAltOutlined,
} from '@ant-design/icons'
import { motion, AnimatePresence } from 'framer-motion'
import { sharingAPI } from '../../services/api'

const { Sider } = Layout

interface SharingStats {
  totalShared: number
  totalReceived: number
  activeShares: number
  pendingShares: number
}

interface SidebarProps {
  onWidthChange?: (width: number) => void
}

function Sidebar({ onWidthChange }: SidebarProps) {
  const navigate = useNavigate()
  const location = useLocation()
  const [collapsed, setCollapsed] = useState(false)
  const [isHovering, setIsHovering] = useState(false)
  const [sharingStats, setSharingStats] = useState<SharingStats>({
    totalShared: 0,
    totalReceived: 0,
    activeShares: 0,
    pendingShares: 0
  })

  // Load sharing statistics
  useEffect(() => {
    const loadSharingStats = async () => {
      try {
        const stats = await sharingAPI.getSharingStats()
        setSharingStats(stats)
      } catch (error) {
        console.error('Failed to load sharing stats:', error)
      }
    }

    loadSharingStats()
    // Refresh stats every 30 seconds
    const interval = setInterval(loadSharingStats, 30000)
    return () => clearInterval(interval)
  }, [])

  const menuItems = [
    {
      key: '/',
      icon: <DashboardOutlined />,
      label: 'Dashboard',
    },
    {
      key: '/containers',
      icon: <FolderOutlined />,
      label: 'Containers',
    },
    {
      key: '/sharing',
      icon: sharingStats.pendingShares > 0 ? (
        <Badge count={sharingStats.pendingShares} size="small">
          <ShareAltOutlined />
        </Badge>
      ) : (
        <ShareAltOutlined />
      ),
      label: 'Sharing',
    },
    {
      key: '/system-info',
      icon: <InfoCircleOutlined />,
      label: 'System Info',
    },
    {
      key: '/settings',
      icon: <SettingOutlined />,
      label: 'Settings',
    },
  ]

  const handleMenuClick = ({ key }: { key: string }) => {
    console.log('Navigation clicked:', key)
    try {
      navigate(key)
      console.log('Navigation successful to:', key)
    } catch (error) {
      console.error('Navigation failed:', error)
    }
  }

  const shouldShowExpanded = !collapsed || isHovering
  const currentWidth = shouldShowExpanded ? 240 : 80

  // Notify parent of width changes
  useEffect(() => {
    if (onWidthChange) {
      onWidthChange(currentWidth)
    }
  }, [currentWidth, onWidthChange])

  return (
    <motion.div
      className="relative"
      onMouseEnter={() => setIsHovering(true)}
      onMouseLeave={() => setIsHovering(false)}
      animate={{
        width: shouldShowExpanded ? 240 : 80,
      }}
      transition={{ duration: 0.3, ease: 'easeInOut' }}
    >
      <Sider
        width={shouldShowExpanded ? 240 : 80}
        collapsed={!shouldShowExpanded}
        className="border-r shadow-xl h-screen"
        theme="light"
        style={{
          background: 'var(--glass-bg)',
          backdropFilter: 'var(--glass-backdrop)',
          borderRight: '1px solid var(--glass-border)',
          height: '100vh',
          position: 'fixed',
          left: 0,
          top: 0,
          zIndex: 100,
          boxShadow: '0 8px 32px var(--glass-shadow)',
        }}
      >
        <motion.div
          initial={{ x: -240 }}
          animate={{ x: 0 }}
          transition={{ duration: 0.3, ease: 'easeOut' }}
          className="h-full flex flex-col relative"
        >
          {/* Collapse Toggle Button */}
          <motion.button
            className="absolute -right-3 top-6 z-10 w-6 h-6 bg-primary-500 hover:bg-primary-600 text-white rounded-full flex items-center justify-center shadow-lg transition-colors duration-200"
            onClick={() => setCollapsed(!collapsed)}
            whileHover={{ scale: 1.1 }}
            whileTap={{ scale: 0.95 }}
          >
            {collapsed ? (
              <MenuUnfoldOutlined className="text-xs" />
            ) : (
              <MenuFoldOutlined className="text-xs" />
            )}
          </motion.button>

          {/* Logo */}
          <div className="h-16 flex items-center justify-center border-b border-gray-200/30 dark:border-gray-700/30">
            <motion.div
              className="flex items-center space-x-3"
              animate={{
                justifyContent: shouldShowExpanded ? 'flex-start' : 'center',
              }}
            >
              <motion.div
                className="w-10 h-10 bg-gradient-to-br from-primary-500 to-primary-600 rounded-xl flex items-center justify-center shadow-lg"
                whileHover={{ scale: 1.05, rotate: 5 }}
                transition={{ duration: 0.2 }}
              >
                <div className="relative">
                  <FolderOutlined className="text-white text-xl" />
                  <LockOutlined className="text-white text-xs absolute -bottom-1 -right-1" />
                </div>
              </motion.div>
              <AnimatePresence>
                {shouldShowExpanded && (
                  <motion.span
                    initial={{ opacity: 0, x: -10 }}
                    animate={{ opacity: 1, x: 0 }}
                    exit={{ opacity: 0, x: -10 }}
                    transition={{ duration: 0.2 }}
                    className="text-xl font-bold bg-gradient-to-r from-primary-600 to-primary-500 bg-clip-text text-transparent"
                  >
                    EFS
                  </motion.span>
                )}
              </AnimatePresence>
            </motion.div>
          </div>

          {/* Navigation Menu */}
          <div className="flex-1 py-6 px-2 flex flex-col">
            <Menu
              mode="inline"
              selectedKeys={[location.pathname]}
              onClick={handleMenuClick}
              className="border-none bg-transparent flex-1"
              inlineCollapsed={!shouldShowExpanded}
              items={menuItems.map((item) => ({
                ...item,
                icon: shouldShowExpanded ? (
                  item.icon
                ) : (
                  <Tooltip title={item.label} placement="right">
                    {item.icon}
                  </Tooltip>
                ),
              }))}
              style={{
                background: 'transparent',
                border: 'none',
                height: '100%',
              }}
            />
          </div>

          {/* Footer */}
          <AnimatePresence>
            {shouldShowExpanded && (
              <motion.div
                initial={{ opacity: 0, y: 20 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: 20 }}
                transition={{ duration: 0.2 }}
                className="p-4 border-t border-gray-200/30 dark:border-gray-700/30"
              >
                <div className="text-xs text-gray-500 dark:text-gray-400 text-center">
                  <div className="font-medium text-gray-700 dark:text-gray-300">
                    Encrypted File System
                  </div>
                  <div className="mt-1 text-primary-500 font-mono">v1.0.0</div>
                </div>
              </motion.div>
            )}
          </AnimatePresence>
        </motion.div>
      </Sider>
    </motion.div>
  )
}

export default Sidebar