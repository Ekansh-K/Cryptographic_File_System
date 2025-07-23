import React, { useState, useEffect } from 'react'
import { Form, Input, Button, Card, Typography, Alert, Progress, Space, Divider } from 'antd'
import { UserOutlined, LockOutlined, EyeInvisibleOutlined, EyeTwoTone, CheckCircleOutlined, CloseCircleOutlined } from '@ant-design/icons'
import { motion, AnimatePresence } from 'framer-motion'
import { authService, RegistrationData, AuthenticationResult } from '../services/authService'
import { useTheme } from '../contexts/ThemeContext'
import ThemeToggle from './ThemeToggle'

const { Title, Text } = Typography

interface RegistrationFormProps {
  onRegistrationSuccess: (user: any) => void
  onSwitchToLogin: () => void
}

interface RegistrationFormData {
  username: string
  password: string
  confirmPassword: string
}

interface PasswordStrength {
  score: number
  feedback: string[]
  color: string
  status: 'exception' | 'normal' | 'success'
}

const RegistrationForm: React.FC<RegistrationFormProps> = ({ onRegistrationSuccess, onSwitchToLogin }) => {
  const [form] = Form.useForm()
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [passwordStrength, setPasswordStrength] = useState<PasswordStrength>({
    score: 0,
    feedback: [],
    color: '#ff4d4f',
    status: 'exception'
  })
  const { isDark } = useTheme()

  // Apply theme to document
  useEffect(() => {
    document.documentElement.setAttribute('data-theme', isDark ? 'dark' : 'light')
  }, [isDark])


  // Password strength calculation
  const calculatePasswordStrength = (password: string): PasswordStrength => {
    if (!password) {
      return {
        score: 0,
        feedback: ['Enter a password'],
        color: '#ff4d4f',
        status: 'exception'
      }
    }

    let score = 0
    const feedback: string[] = []

    // Length check
    if (password.length >= 8) {
      score += 25
    } else {
      feedback.push('At least 8 characters')
    }

    // Uppercase check
    if (/[A-Z]/.test(password)) {
      score += 25
    } else {
      feedback.push('One uppercase letter')
    }

    // Lowercase check
    if (/[a-z]/.test(password)) {
      score += 25
    } else {
      feedback.push('One lowercase letter')
    }

    // Number check
    if (/\d/.test(password)) {
      score += 12.5
    } else {
      feedback.push('One number')
    }

    // Special character check
    if (/[@$!%*?&]/.test(password)) {
      score += 12.5
    } else {
      feedback.push('One special character (@$!%*?&)')
    }

    let color = '#ff4d4f'
    let status: 'exception' | 'normal' | 'success' = 'exception'

    if (score >= 100) {
      color = '#52c41a'
      status = 'success'
    } else if (score >= 75) {
      color = '#faad14'
      status = 'normal'
    } else if (score >= 50) {
      color = '#fa8c16'
      status = 'normal'
    }

    return {
      score,
      feedback: feedback.length > 0 ? feedback : ['Strong password!'],
      color,
      status
    }
  }

  // Watch password field changes
  useEffect(() => {
    const password = form.getFieldValue('password')
    if (password !== undefined) {
      setPasswordStrength(calculatePasswordStrength(password))
    }
  }, [form])

  // Clear error when form values change
  useEffect(() => {
    if (error) {
      setError(null)
    }
  }, [form.getFieldsValue(), error])

  const handleRegistration = async (values: RegistrationFormData) => {
    setLoading(true)
    setError(null)

    try {
      const registrationData: RegistrationData = {
        username: values.username.trim(),
        password: values.password,
        confirmPassword: values.confirmPassword
      }

      const result: AuthenticationResult = await authService.register(registrationData)

      if (result.success && result.user) {
        onRegistrationSuccess(result.user)
      } else {
        setError(result.error || 'Registration failed')
      }
    } catch (err) {
      setError('An unexpected error occurred. Please try again.')
      console.error('Registration error:', err)
    } finally {
      setLoading(false)
    }
  }

  const handleFormSubmit = (values: RegistrationFormData) => {
    handleRegistration(values)
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

  const strengthVariants = {
    hidden: { opacity: 0, height: 0 },
    visible: { 
      opacity: 1, 
      height: 'auto',
      transition: { duration: 0.3 }
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
                Create Account
              </Title>
              <Text type="secondary" className="text-base">
                Join EFS to secure your files with encryption
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
              name="registration"
              onFinish={handleFormSubmit}
              layout="vertical"
              size="large"
              onValuesChange={() => {
                const password = form.getFieldValue('password')
                if (password !== undefined) {
                  setPasswordStrength(calculatePasswordStrength(password))
                }
              }}
            >
              <Form.Item
                name="username"
                label="Username"
                rules={[
                  { required: true, message: 'Please enter a username' },
                  { min: 3, message: 'Username must be at least 3 characters' },
                  { max: 20, message: 'Username must be less than 20 characters' },
                  { 
                    pattern: /^[a-zA-Z0-9_-]+$/, 
                    message: 'Username can only contain letters, numbers, underscores, and hyphens' 
                  }
                ]}
              >
                <Input
                  prefix={<UserOutlined className="text-gray-400" />}
                  placeholder="Choose a username"
                  className="rounded-lg h-12"
                  autoComplete="username"
                />
              </Form.Item>

              <Form.Item
                name="password"
                label="Password"
                rules={[
                  { required: true, message: 'Please enter a password' },
                  { min: 8, message: 'Password must be at least 8 characters' },
                  {
                    validator: (_, value) => {
                      if (!value) return Promise.resolve()
                      const strength = calculatePasswordStrength(value)
                      if (strength.score < 75) {
                        return Promise.reject(new Error('Password is too weak'))
                      }
                      return Promise.resolve()
                    }
                  }
                ]}
              >
                <Input.Password
                  prefix={<LockOutlined className="text-gray-400" />}
                  placeholder="Create a strong password"
                  className="rounded-lg h-12"
                  autoComplete="new-password"
                  iconRender={(visible) => 
                    visible ? <EyeTwoTone /> : <EyeInvisibleOutlined />
                  }
                />
              </Form.Item>

              {/* Password Strength Indicator */}
              <AnimatePresence>
                {form.getFieldValue('password') && (
                  <motion.div
                    variants={strengthVariants}
                    initial="hidden"
                    animate="visible"
                    exit="hidden"
                    className="mb-4"
                  >
                    <div className="mb-2">
                      <Progress
                        percent={passwordStrength.score}
                        strokeColor={passwordStrength.color}
                        status={passwordStrength.status}
                        showInfo={false}
                        size="small"
                      />
                    </div>
                    <div className="text-xs">
                      <Space direction="vertical" size={2}>
                        {passwordStrength.feedback.map((item, index) => (
                          <div key={index} className="flex items-center gap-1">
                            {passwordStrength.score >= 75 && passwordStrength.feedback.includes('Strong password!') ? (
                              <CheckCircleOutlined style={{ color: '#52c41a' }} />
                            ) : (
                              <CloseCircleOutlined style={{ color: '#ff4d4f' }} />
                            )}
                            <Text type="secondary" className="text-xs">
                              {item}
                            </Text>
                          </div>
                        ))}
                      </Space>
                    </div>
                  </motion.div>
                )}
              </AnimatePresence>

              <Form.Item
                name="confirmPassword"
                label="Confirm Password"
                dependencies={['password']}
                rules={[
                  { required: true, message: 'Please confirm your password' },
                  ({ getFieldValue }) => ({
                    validator(_, value) {
                      if (!value || getFieldValue('password') === value) {
                        return Promise.resolve()
                      }
                      return Promise.reject(new Error('Passwords do not match'))
                    },
                  }),
                ]}
              >
                <Input.Password
                  prefix={<LockOutlined className="text-gray-400" />}
                  placeholder="Confirm your password"
                  className="rounded-lg h-12"
                  autoComplete="new-password"
                  iconRender={(visible) => 
                    visible ? <EyeTwoTone /> : <EyeInvisibleOutlined />
                  }
                />
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
                  {loading ? 'Creating Account...' : 'Create Account'}
                </Button>
              </Form.Item>
            </Form>

            <Divider className="my-6">
              <Text type="secondary" className="text-sm">
                Already have an account?
              </Text>
            </Divider>

            <div className="text-center">
              <Button
                type="link"
                onClick={onSwitchToLogin}
                className="text-base font-medium p-0 h-auto"
                style={{ color: '#0ea5e9' }}
              >
                Sign in instead
              </Button>
            </div>
          </motion.div>
        </Card>
      </motion.div>
    </div>
  )
}

export default RegistrationForm