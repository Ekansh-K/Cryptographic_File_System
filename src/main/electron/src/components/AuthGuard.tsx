import React, { useEffect, useState, useCallback } from 'react'
import { Spin, message } from 'antd'
import { motion } from 'framer-motion'
import { useNavigate } from 'react-router-dom'
import { authService } from '../services/authService'
import LoginPage from '../pages/LoginPage'
import RegistrationForm from './RegistrationForm'

interface AuthGuardProps {
  children: React.ReactNode
}

type AuthView = 'login' | 'register'

const AuthGuard: React.FC<AuthGuardProps> = ({ children }) => {
  const [isAuthenticated, setIsAuthenticated] = useState<boolean | null>(null)
  const [currentView, setCurrentView] = useState<AuthView>('login')
  const [isLoading, setIsLoading] = useState(true)
  const [sessionCheckInterval, setSessionCheckInterval] = useState<NodeJS.Timeout | null>(null)
  const navigate = useNavigate()

  // Check authentication status
  const checkAuthStatus = useCallback(async () => {
    try {
      setIsLoading(true)
      
      // Check if user is already authenticated
      if (authService.isAuthenticated()) {
        setIsAuthenticated(true)
        return
      }

      // Try to restore session from storage
      const session = await authService.restoreSession()
      if (session) {
        setIsAuthenticated(true)
        // Update activity to keep session alive
        authService.updateActivity()
      } else {
        setIsAuthenticated(false)
      }
    } catch (error) {
      console.error('Auth check failed:', error)
      setIsAuthenticated(false)
    } finally {
      setIsLoading(false)
    }
  }, [])

  // Set up periodic session validation
  const setupSessionMonitoring = useCallback(() => {
    // Clear existing interval
    if (sessionCheckInterval) {
      clearInterval(sessionCheckInterval)
    }

    // Check session every 30 seconds
    const interval = setInterval(async () => {
      try {
        if (!authService.isAuthenticated()) {
          console.log('Session expired, redirecting to login')
          setIsAuthenticated(false)
          message.warning('Your session has expired. Please log in again.')
          navigate('/login')
          return
        }

        // Update activity timestamp
        authService.updateActivity()

        // Verify credential integrity periodically
        const integrityCheck = await authService.verifyCredentialIntegrity()
        if (!integrityCheck) {
          console.warn('Credential integrity check failed')
          await authService.logout()
          setIsAuthenticated(false)
          message.error('Security check failed. Please log in again.')
          navigate('/login')
        }
      } catch (error) {
        console.error('Session monitoring error:', error)
      }
    }, 30000) // 30 seconds

    setSessionCheckInterval(interval)
  }, [sessionCheckInterval, navigate])

  // Clean up session monitoring
  const cleanupSessionMonitoring = useCallback(() => {
    if (sessionCheckInterval) {
      clearInterval(sessionCheckInterval)
      setSessionCheckInterval(null)
    }
  }, [sessionCheckInterval])

  useEffect(() => {
    checkAuthStatus()
  }, [checkAuthStatus])

  useEffect(() => {
    if (isAuthenticated) {
      setupSessionMonitoring()
    } else {
      cleanupSessionMonitoring()
    }

    // Cleanup on unmount
    return () => {
      cleanupSessionMonitoring()
    }
  }, [isAuthenticated, setupSessionMonitoring, cleanupSessionMonitoring])

  // Handle successful login
  const handleLoginSuccess = useCallback(async (user: any) => {
    console.log('Login successful:', user)
    setIsAuthenticated(true)
    message.success(`Welcome back, ${user.username}!`)
    
    // Navigate to dashboard after successful login
    navigate('/')
  }, [navigate])

  // Handle successful registration
  const handleRegistrationSuccess = useCallback(async (user: any) => {
    console.log('Registration successful:', user)
    setIsAuthenticated(true)
    message.success(`Welcome, ${user.username}! Your account has been created.`)
    
    // Navigate to dashboard after successful registration
    navigate('/')
  }, [navigate])

  // Handle logout
  const handleLogout = useCallback(async () => {
    try {
      await authService.logout()
      setIsAuthenticated(false)
      cleanupSessionMonitoring()
      message.info('You have been logged out successfully.')
      navigate('/login')
    } catch (error) {
      console.error('Logout failed:', error)
      message.error('Logout failed. Please try again.')
    }
  }, [navigate, cleanupSessionMonitoring])

  // Handle view switching
  const handleSwitchToRegister = useCallback(() => {
    setCurrentView('register')
  }, [])

  const handleSwitchToLogin = useCallback(() => {
    setCurrentView('login')
  }, [])

  // Handle session expiration
  useEffect(() => {
    const handleSessionExpired = () => {
      setIsAuthenticated(false)
      message.warning('Your session has expired. Please log in again.')
      navigate('/login')
    }

    // Listen for session expiration events
    window.addEventListener('sessionExpired', handleSessionExpired)
    
    return () => {
      window.removeEventListener('sessionExpired', handleSessionExpired)
    }
  }, [navigate])

  // Loading state
  if (isLoading) {
    return (
      <div className="min-h-screen flex items-center justify-center"
           style={{
             background: 'linear-gradient(135deg, #667eea 0%, #764ba2 100%)'
           }}>
        <motion.div
          initial={{ opacity: 0, scale: 0.8 }}
          animate={{ opacity: 1, scale: 1 }}
          transition={{ duration: 0.5 }}
          className="text-center"
        >
          <Spin size="large" />
          <div className="mt-4 text-white text-lg font-medium">
            Checking authentication...
          </div>
        </motion.div>
      </div>
    )
  }

  // Not authenticated - show login/register
  if (!isAuthenticated) {
    return (
      <motion.div
        key={currentView}
        initial={{ opacity: 0, x: currentView === 'login' ? -50 : 50 }}
        animate={{ opacity: 1, x: 0 }}
        exit={{ opacity: 0, x: currentView === 'login' ? 50 : -50 }}
        transition={{ duration: 0.3 }}
      >
        {currentView === 'login' ? (
          <LoginPage
            onLoginSuccess={handleLoginSuccess}
            onSwitchToRegister={handleSwitchToRegister}
          />
        ) : (
          <RegistrationForm
            onRegistrationSuccess={handleRegistrationSuccess}
            onSwitchToLogin={handleSwitchToLogin}
          />
        )}
      </motion.div>
    )
  }

  // Authenticated - show protected content
  return <>{children}</>
}

export default AuthGuard