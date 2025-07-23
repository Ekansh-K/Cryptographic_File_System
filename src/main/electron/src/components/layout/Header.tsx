import { useState, useEffect } from 'react'
import { Layout, Button, Dropdown, Space, Avatar, Typography, Modal, Input, List, Empty, message } from 'antd'
import {
  UserOutlined,
  SunOutlined,
  MoonOutlined,
  DesktopOutlined,
  SettingOutlined,
  LogoutOutlined,
  SearchOutlined,
  QuestionCircleOutlined,
} from '@ant-design/icons'
import { useTheme } from '../../contexts/ThemeContext'
import { useNavigate, useLocation } from 'react-router-dom'
import { motion } from 'framer-motion'
import { authService } from '../../services/authService'

const { Header: AntHeader } = Layout
const { Text } = Typography

interface HeaderProps {
  sidebarWidth?: number
}

function Header({ sidebarWidth = 80 }: HeaderProps) {
  const { theme, setTheme } = useTheme()
  const navigate = useNavigate()
  const location = useLocation()
  const [currentTime, setCurrentTime] = useState(new Date())
  const [searchVisible, setSearchVisible] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const [currentUser] = useState(authService.getCurrentUser())

  // Log sidebar width for debugging (can be removed in production)
  console.log('Header rendered with sidebar width:', sidebarWidth)


  useEffect(() => {
    const timer = setInterval(() => {
      setCurrentTime(new Date())
    }, 1000)
    return () => clearInterval(timer)
  }, [])

  const getPageTitle = () => {
    switch (location.pathname) {
      case '/':
        return 'Dashboard'
      case '/containers':
        return 'Container Management'
      case '/system-info':
        return 'System Information'
      case '/settings':
        return 'Settings'
      default:
        return 'Encrypted File System'
    }
  }

  const themeMenuItems = [
    {
      key: 'light',
      icon: <SunOutlined />,
      label: 'Light Theme',
      onClick: () => setTheme('light'),
    },
    {
      key: 'dark',
      icon: <MoonOutlined />,
      label: 'Dark Theme',
      onClick: () => setTheme('dark'),
    },
    {
      key: 'system',
      icon: <DesktopOutlined />,
      label: 'System Theme',
      onClick: () => setTheme('system'),
    },
  ]

  // Handle logout
  const handleLogout = async () => {
    try {
      await authService.logout()
      message.success('Logged out successfully')
      // Reload the page to trigger authentication check
      window.location.reload()
    } catch (error) {
      console.error('Logout failed:', error)
      message.error('Logout failed. Please try again.')
    }
  }

  const userMenuItems = [
    {
      key: 'profile',
      icon: <UserOutlined />,
      label: 'Profile',
      onClick: () => navigate('/profile'),
    },
    {
      key: 'settings',
      icon: <SettingOutlined />,
      label: 'Settings',
      onClick: () => navigate('/settings'),
    },
    {
      key: 'help',
      icon: <QuestionCircleOutlined />,
      label: 'Help & Support',
    },
    {
      type: 'divider' as const,
    },
    {
      key: 'logout',
      icon: <LogoutOutlined />,
      label: 'Sign Out',
      danger: true,
      onClick: handleLogout,
    },
  ]

  const getCurrentThemeIcon = () => {
    switch (theme) {
      case 'light':
        return <SunOutlined />
      case 'dark':
        return <MoonOutlined />
      default:
        return <DesktopOutlined />
    }
  }

  // Mock search results
  const getSearchResults = (query: string) => {
    const allItems = [
      { type: 'container', title: 'My Documents', description: 'Personal files container', path: '/containers' },
      { type: 'container', title: 'Work Files', description: 'Work related documents', path: '/containers' },
      { type: 'container', title: 'Photos', description: 'Image collection', path: '/containers' },
      { type: 'setting', title: 'Security Settings', description: 'Configure security options', path: '/settings' },
      { type: 'setting', title: 'Theme Settings', description: 'Customize appearance', path: '/settings' },
      { type: 'setting', title: 'Performance Settings', description: 'Optimize performance', path: '/settings' },
      { type: 'page', title: 'Dashboard', description: 'Main overview page', path: '/' },
      { type: 'page', title: 'Container Management', description: 'Manage all containers', path: '/containers' },
    ]

    if (!query) return allItems.slice(0, 5)

    return allItems.filter(item =>
      item.title.toLowerCase().includes(query.toLowerCase()) ||
      item.description.toLowerCase().includes(query.toLowerCase())
    )
  }



  const handleSearch = (query: string) => {
    setSearchQuery(query)
  }

  const handleSearchResultClick = (item: any) => {
    navigate(item.path)
    setSearchVisible(false)
    setSearchQuery('')
  }

  return (
    <AntHeader
      className="glass-card border-0 shadow-lg flex items-center justify-between transition-all duration-300 ease-in-out"
      style={{
        background: 'var(--glass-bg)',
        backdropFilter: 'var(--glass-backdrop)',
        borderBottom: '1px solid var(--glass-border)',
        paddingLeft: '24px',
        paddingRight: '24px',
        width: '100%',
        height: '64px',
        position: 'relative',
        zIndex: 30,
        boxShadow: '0 4px 20px var(--glass-shadow)',
        margin: 0,
        borderRadius: '0px',
      }}
    >
      <motion.div
        initial={{ opacity: 0, x: -20 }}
        animate={{ opacity: 1, x: 0 }}
        transition={{ duration: 0.3 }}
        className="flex items-center space-x-6 h-full"
      >
        <div className="flex flex-col justify-center h-full py-2">
          <h1 className="text-xl font-bold text-gray-800 dark:text-gray-200 m-0 leading-tight">
            {getPageTitle()}
          </h1>
          <Text className="text-sm text-gray-500 dark:text-gray-400 leading-tight">
            {currentTime.toLocaleString()}
          </Text>
        </div>
      </motion.div>

      <motion.div
        initial={{ opacity: 0, x: 20 }}
        animate={{ opacity: 1, x: 0 }}
        transition={{ duration: 0.3, delay: 0.1 }}
      >
        <Space size="middle">
          {/* Search */}
          <motion.div whileHover={{ scale: 1.05 }} whileTap={{ scale: 0.95 }}>
            <Button
              type="text"
              icon={<SearchOutlined />}
              className="flex items-center justify-center hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg"
              size="large"
              onClick={() => setSearchVisible(true)}
            />
          </motion.div>



          {/* Theme Switcher */}
          <motion.div whileHover={{ scale: 1.05 }} whileTap={{ scale: 0.95 }}>
            <Dropdown
              menu={{ items: themeMenuItems }}
              trigger={['click']}
              placement="bottomRight"
            >
              <Button
                type="text"
                icon={getCurrentThemeIcon()}
                className="flex items-center justify-center hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg"
                size="large"
              />
            </Dropdown>
          </motion.div>

          {/* User Menu */}
          <motion.div whileHover={{ scale: 1.05 }} whileTap={{ scale: 0.95 }}>
            <Dropdown
              menu={{ items: userMenuItems }}
              trigger={['click']}
              placement="bottomRight"
            >
              <div className="flex items-center space-x-2 cursor-pointer hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg p-2 transition-colors">
                <Avatar
                  size="small"
                  icon={<UserOutlined />}
                  className="bg-gradient-to-br from-primary-500 to-primary-600"
                />
                <div className="hidden md:block">
                  <Text className="text-sm font-medium text-gray-700 dark:text-gray-300">
                    {currentUser?.username || 'User'}
                  </Text>
                </div>
              </div>
            </Dropdown>
          </motion.div>
        </Space>
      </motion.div>

      {/* Global Search Modal */}
      <Modal
        title="Global Search"
        open={searchVisible}
        onCancel={() => setSearchVisible(false)}
        footer={null}
        width={600}
      >
        <Input
          placeholder="Search containers, settings, and more..."
          value={searchQuery}
          onChange={(e) => handleSearch(e.target.value)}
          size="large"
          prefix={<SearchOutlined />}
          autoFocus
        />
        <div className="mt-4 max-h-96 overflow-y-auto">
          {getSearchResults(searchQuery).length > 0 ? (
            <List
              dataSource={getSearchResults(searchQuery)}
              renderItem={(item) => (
                <List.Item
                  className="cursor-pointer hover:bg-gray-50 dark:hover:bg-gray-800 rounded-lg px-3 py-2"
                  onClick={() => handleSearchResultClick(item)}
                >
                  <List.Item.Meta
                    title={item.title}
                    description={item.description}
                    avatar={
                      <div className={`w-8 h-8 rounded-full flex items-center justify-center ${item.type === 'container' ? 'bg-blue-100 text-blue-600' :
                          item.type === 'setting' ? 'bg-green-100 text-green-600' :
                            'bg-purple-100 text-purple-600'
                        }`}>
                        {item.type === 'container' ? '📁' :
                          item.type === 'setting' ? '⚙️' : '📄'}
                      </div>
                    }
                  />
                </List.Item>
              )}
            />
          ) : (
            <Empty description="No results found" />
          )}
        </div>
      </Modal>


    </AntHeader>
  )
}

export default Header