import { useEffect, useState } from 'react'
import { HashRouter as Router, Routes, Route, useLocation, Navigate } from 'react-router-dom'
import { ConfigProvider, theme } from 'antd'
import { ThemeProvider, useTheme } from './contexts/ThemeContext'
import Sidebar from './components/layout/Sidebar'
import Header from './components/layout/Header'
import Dashboard from './pages/Dashboard'
import Containers from './pages/Containers'
import Settings from './pages/Settings'
import SystemInfo from './pages/SystemInfo'
import Sharing from './pages/Sharing'
import LoginPage from './pages/LoginPage'
import AuthGuard from './components/AuthGuard'
import { motion } from 'framer-motion'
import { authService } from './services/authService'





// Protected route wrapper component
function ProtectedRoute({ children }: { children: React.ReactNode }) {
  return (
    <AuthGuard>
      {children}
    </AuthGuard>
  )
}

// Main application layout for authenticated users
function AuthenticatedLayout() {
  const { isDark } = useTheme()
  const [sidebarWidth, setSidebarWidth] = useState(80)

  useEffect(() => {
    // Apply theme class to document
    if (isDark) {
      document.documentElement.classList.add('dark')
      document.documentElement.setAttribute('data-theme', 'dark')
    } else {
      document.documentElement.classList.remove('dark')
      document.documentElement.setAttribute('data-theme', 'light')
    }
  }, [isDark])

  return (
    <div className="min-h-screen flex bg-transparent">
      <Sidebar onWidthChange={setSidebarWidth} />
      
      {/* Main content container with responsive margins */}
      <div 
        className="flex-1 flex flex-col min-h-screen transition-all duration-300 ease-in-out"
        style={{ 
          marginLeft: `${sidebarWidth}px`, // Sidebar width only
          width: `calc(100vw - ${sidebarWidth}px)`, // Full remaining width
        }}
      >
        {/* Header - sticky positioned */}
        <div className="flex-shrink-0 sticky top-0 z-30 w-full">
          <Header sidebarWidth={sidebarWidth} />
        </div>

        {/* Main content area with reduced left padding */}
        <main 
          className="flex-1 overflow-auto bg-transparent"
          style={{ 
            paddingTop: '24px',
            paddingRight: '24px', 
            paddingBottom: '24px',
            paddingLeft: '12px', // Reduced left padding by half
            minHeight: 'calc(100vh - 64px)'
          }}
        >
          <motion.div
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.3 }}
            className="h-full max-w-full"
          >
            <Routes>
              <Route path="/" element={<Dashboard />} />
              <Route path="/containers" element={<Containers />} />
              <Route path="/sharing" element={<Sharing />} />
              <Route path="/system-info" element={<SystemInfo />} />
              <Route path="/settings" element={<Settings />} />
              {/* Redirect any unknown routes to dashboard */}
              <Route path="*" element={<Navigate to="/" replace />} />
            </Routes>
          </motion.div>
        </main>
      </div>
    </div>
  )
}

function AppContent() {
  const { isDark } = useTheme()
  const [isAuthenticated, setIsAuthenticated] = useState<boolean | null>(null)
  const [currentUser, setCurrentUser] = useState<any>(null)

  useEffect(() => {
    // Check authentication status on app load
    checkAuthStatus()
  }, [])

  const checkAuthStatus = async () => {
    try {
      // Try to restore session first
      const session = await authService.restoreSession()
      if (session) {
        setIsAuthenticated(true)
      } else {
        setIsAuthenticated(authService.isAuthenticated())
      }
    } catch (error) {
      console.error('Auth check failed:', error)
      setIsAuthenticated(false)
    }
  }

  // Show loading state while checking authentication
  if (isAuthenticated === null) {
    return (
      <ConfigProvider
        theme={{
          algorithm: isDark ? theme.darkAlgorithm : theme.defaultAlgorithm,
          token: {
            colorPrimary: '#0ea5e9',
            borderRadius: 8,
            fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif',
          },
        }}
      >
        <div className="min-h-screen flex items-center justify-center"
             style={{
               background: 'linear-gradient(135deg, #667eea 0%, #764ba2 100%)'
             }}>
          <motion.div
            initial={{ opacity: 0, scale: 0.8 }}
            animate={{ opacity: 1, scale: 1 }}
            transition={{ duration: 0.5 }}
            className="text-center text-white"
          >
            <div className="text-lg font-medium">Loading...</div>
          </motion.div>
        </div>
      </ConfigProvider>
    )
  }

  return (
    <ConfigProvider
      theme={{
        algorithm: isDark ? theme.darkAlgorithm : theme.defaultAlgorithm,
        token: {
          colorPrimary: '#0ea5e9',
          borderRadius: 8,
          fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif',
        },
      }}
    >
      <Router>
        <Routes>
          {/* Authentication routes - accessible when not authenticated */}
          <Route 
            path="/login" 
            element={
              isAuthenticated ? <Navigate to="/" replace /> : (
                <LoginPage 
                  onLoginSuccess={(user) => {
                    setIsAuthenticated(true)
                    setCurrentUser(user)
                  }}
                  onSwitchToRegister={() => {
                    // Handle switch to register - could navigate to register page
                    console.log('Switch to register')
                  }}
                />
              )
            } 
          />
          
          {/* Protected routes - require authentication */}
          <Route 
            path="/*" 
            element={
              <ProtectedRoute>
                <AuthenticatedLayout />
              </ProtectedRoute>
            } 
          />
        </Routes>
      </Router>
    </ConfigProvider>
  )
}

function App() {
  return (
    <ThemeProvider>
      <AppContent />
    </ThemeProvider>
  )
}

export default App