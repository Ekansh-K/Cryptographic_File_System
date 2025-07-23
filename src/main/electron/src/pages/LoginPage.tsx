import React, { useState, useEffect } from 'react'
import { Form, Input, Button, Card, Typography, Alert, Checkbox, Divider } from 'antd'
import { UserOutlined, LockOutlined, EyeInvisibleOutlined, EyeTwoTone } from '@ant-design/icons'
import { motion, AnimatePresence } from 'framer-motion'
import { authService, LoginCredentials, AuthenticationResult } from '../services/authService'
import { useTheme } from '../contexts/ThemeContext'
import ThemeToggle from '../components/ThemeToggle'

const { Title, Text } = Typography

interface LoginPageProps {
  onLoginSuccess: (user: any) => void
  onSwitchToRegister: () => void
}

interface LoginFormData {
  username: string
  password: string
  rememberMe: boolean
}

const LoginPage: React.FC<LoginPageProps> = ({ onLoginSuccess, onSwitchToRegister }) => {
  const [form] = Form.useForm()
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const { isDark } = useTheme()

  // Clear error when form values change
  useEffect(() => {
    if (error) {
      setError(null)
    }
  }, [form, error])

  // Apply theme to document
  useEffect(() => {
    document.documentElement.setAttribute('data-theme', isDark ? 'dark' : 'light')
  }, [isDark])

  const handleLogin = async (values: LoginFormData) => {
    setLoading(true)
    setError(null)

    try {
      const credentials: LoginCredentials = {
        username: values.username.trim(),
        password: values.password,
        rememberMe: values.rememberMe
      }

      const result: AuthenticationResult = await authService.login(credentials)

      if (result.success && result.user) {
        onLoginSuccess(result.user)
      } else {
        if (result.requiresRegistration) {
          setError('User not found. Please register first.')
        } else {
          setError(result.error || 'Login failed')
        }
      }
    } catch (err) {
      setError('An unexpected error occurred. Please try again.')
      console.error('Login error:', err)
    } finally {
      setLoading(false)
    }
  }

  const handleFormSubmit = (values: LoginFormData) => {
    handleLogin(values)
  }

  const cardVariants = {
    hidden: { 
      opacity: 0, 
      y: 50,
      scale: 0.9
    },
    visible: { 
      opacity: 1, 
      y: 0,
      scale: 1,
      transition: {
        duration: 0.6,
        ease: "easeOut"
      }
    }
  }

  const formVariants = {
    hidden: { opacity: 0 },
    visible: { 
      opacity: 1,
      transition: {
        delay: 0.3,
        duration: 0.4
      }
    }
  }

  return (
    <div 
      className={`min-h-screen flex items-center justify-center p-4 relative overflow-hidden transition-all duration-500 ${
        isDark ? 'auth-bg-dark' : 'auth-bg-light'
      }`}
    >
      {/* Theme Toggle */}
      <div className="absolute top-6 right-6 z-50">
        <ThemeToggle size="large" />
      </div>
      
      {/* Enhanced Animated background particles */}
      <div className="absolute inset-0 overflow-hidden">
        {[...Array(30)].map((_, i) => (
          <motion.div
            key={i}
            className={`absolute rounded-full ${
              isDark 
                ? 'bg-gradient-to-r from-blue-400 to-purple-400' 
                : 'bg-gradient-to-r from-blue-300 to-cyan-300'
            }`}
            style={{
              width: Math.random() * 6 + 2 + 'px',
              height: Math.random() * 6 + 2 + 'px',
              left: `${Math.random() * 100}%`,
              top: `${Math.random() * 100}%`,
            }}
            animate={{
              x: [0, Math.random() * 200 - 100, 0],
              y: [0, Math.random() * 200 - 100, 0],
              opacity: [0.1, 0.6, 0.1],
              scale: [1, 1.5, 1],
            }}
            transition={{
              duration: 8 + Math.random() * 10,
              repeat: Infinity,
              ease: "easeInOut",
              delay: Math.random() * 5,
            }}
          />
        ))}
      </div>

      {/* Floating geometric shapes */}
      <div className="absolute inset-0 overflow-hidden pointer-events-none">
        {[...Array(8)].map((_, i) => (
          <motion.div
            key={`shape-${i}`}
            className={`absolute ${
              isDark 
                ? 'border-blue-400/20' 
                : 'border-blue-300/30'
            } border rounded-full`}
            style={{
              width: Math.random() * 100 + 50 + 'px',
              height: Math.random() * 100 + 50 + 'px',
              left: `${Math.random() * 100}%`,
              top: `${Math.random() * 100}%`,
            }}
            animate={{
              rotate: [0, 360],
              scale: [1, 1.2, 1],
              opacity: [0.1, 0.3, 0.1],
            }}
            transition={{
              duration: 15 + Math.random() * 10,
              repeat: Infinity,
              ease: "linear",
            }}
          />
        ))}
      </div>

      <motion.div
        variants={cardVariants}
        initial="hidden"
        animate="visible"
        className="w-full max-w-md relative z-10"
      >
        <Card
          className="glass-card border-0 shadow-2xl overflow-hidden"
          style={{
            background: isDark 
              ? 'rgba(15, 23, 42, 0.8)' 
              : 'rgba(255, 255, 255, 0.9)',
            backdropFilter: 'blur(32px) saturate(180%)',
            borderRadius: '24px',
            border: isDark 
              ? '1px solid rgba(148, 163, 184, 0.2)' 
              : '1px solid rgba(255, 255, 255, 0.3)',
            boxShadow: isDark
              ? '0 25px 50px rgba(0, 0, 0, 0.5), inset 0 1px 0 rgba(255, 255, 255, 0.1)'
              : '0 25px 50px rgba(0, 0, 0, 0.15), inset 0 1px 0 rgba(255, 255, 255, 0.2)',
          }}
        >
          <motion.div variants={formVariants} initial="hidden" animate="visible">
            <div className="text-center mb-8">
              <Title level={2} className="text-gradient mb-2">
                Welcome Back
              </Title>
              <Text type="secondary" className="text-base">
                Sign in to access your encrypted containers
              </Text>
            </div>

            <AnimatePresence>
              {error && (
                <motion.div
                  initial={{ opacity: 0, height: 0 }}
                  animate={{ opacity: 1, height: 'auto' }}
                  exit={{ opacity: 0, height: 0 }}
                  transition={{ duration: 0.3 }}
                  className="mb-4"
                >
                  <Alert
                    message={error}
                    type="error"
                    showIcon
                    closable
                    onClose={() => setError(null)}
                    className="rounded-lg"
                  />
                </motion.div>
              )}
            </AnimatePresence>

            <Form
              form={form}
              name="login"
              onFinish={handleFormSubmit}
              layout="vertical"
              size="large"
              initialValues={{ rememberMe: false }}
            >
              <Form.Item
                name="username"
                label="Username"
                rules={[
                  { required: true, message: 'Please enter your username' },
                  { min: 3, message: 'Username must be at least 3 characters' }
                ]}
              >
                <Input
                  prefix={<UserOutlined className="text-gray-400" />}
                  placeholder="Enter your username"
                  className="rounded-lg h-12"
                  autoComplete="username"
                />
              </Form.Item>

              <Form.Item
                name="password"
                label="Password"
                rules={[
                  { required: true, message: 'Please enter your password' }
                ]}
              >
                <Input.Password
                  prefix={<LockOutlined className="text-gray-400" />}
                  placeholder="Enter your password"
                  className="rounded-lg h-12"
                  autoComplete="current-password"
                  iconRender={(visible) => 
                    visible ? <EyeTwoTone /> : <EyeInvisibleOutlined />
                  }
                />
              </Form.Item>

              <Form.Item name="rememberMe" valuePropName="checked">
                <Checkbox className="text-sm">
                  Remember me for 7 days
                </Checkbox>
              </Form.Item>

              <Form.Item className="mb-4">
                <Button
                  type="primary"
                  htmlType="submit"
                  loading={loading}
                  className="w-full h-12 rounded-lg font-semibold text-base"
                  style={{
                    background: 'linear-gradient(135deg, #0ea5e9 0%, #06b6d4 100%)',
                    border: 'none',
                    boxShadow: '0 4px 15px rgba(14, 165, 233, 0.3)'
                  }}
                >
                  {loading ? 'Signing In...' : 'Sign In'}
                </Button>
              </Form.Item>
            </Form>

            <Divider className="my-6">
              <Text type="secondary" className="text-sm">
                New to EFS?
              </Text>
            </Divider>

            <div className="text-center">
              <Button
                type="link"
                onClick={onSwitchToRegister}
                className="text-base font-medium p-0 h-auto"
                style={{ color: '#0ea5e9' }}
              >
                Create an account
              </Button>
            </div>
          </motion.div>
        </Card>
      </motion.div>
    </div>
  )
}

export default LoginPage