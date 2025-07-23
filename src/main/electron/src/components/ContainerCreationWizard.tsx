import { useState, useCallback } from 'react'
import {
  Modal,
  Steps,
  Form,
  Input,
  Select,
  Slider,
  Switch,
  Button,
  Space,
  Card,
  Row,
  Col,
  Progress,
  Alert,
  Tooltip,
  Upload,
  Typography,
  Divider,
  Collapse,
  InputNumber,
  Radio,
  message,
} from 'antd'
import {
  InfoCircleOutlined,
  LockOutlined,
  SafetyOutlined,
  EyeInvisibleOutlined,
  UploadOutlined,
  QuestionCircleOutlined,
  SettingOutlined,
  CheckCircleOutlined,
  LoadingOutlined,
} from '@ant-design/icons'
import { motion, AnimatePresence } from 'framer-motion'
import { containerAPI, type ContainerConfig } from '../services/api'

const { Title, Text } = Typography
const { Dragger } = Upload

interface ContainerCreationWizardProps {
  visible: boolean
  onClose: () => void
  onSuccess: () => void
}

interface WizardFormData {
  // Basic Configuration
  name: string
  size: number
  password: string
  confirmPassword: string
  
  // Encryption Settings
  encryptionAlgorithm: string
  keyDerivationIterations: number
  
  // Steganography Settings
  steganographic: boolean
  carrierFile?: File
  carrierType?: string
  embeddingDensity: number
  
  // Advanced Options
  compressionEnabled: boolean
  integrityChecking: boolean
  keyRotationEnabled: boolean
  keyRotationInterval: number
}

const ENCRYPTION_ALGORITHMS = [
  { value: 'AES-256-GCM', label: 'AES-256-GCM (Recommended)', description: 'Industry standard with authentication' },
  { value: 'AES-256-CBC', label: 'AES-256-CBC', description: 'Traditional AES encryption' },
  { value: 'ChaCha20-Poly1305', label: 'ChaCha20-Poly1305', description: 'Modern alternative to AES' },
]

const CARRIER_TYPES = [
  { value: 'PNG', label: 'PNG Image', description: 'Best for lossless embedding' },
  { value: 'JPEG', label: 'JPEG Image', description: 'Good for photos' },
  { value: 'BMP', label: 'BMP Image', description: 'Maximum capacity' },
  { value: 'WAV', label: 'WAV Audio', description: 'Audio steganography' },
]

const SIZE_PRESETS = [
  { label: '100 MB', value: 100 },
  { label: '500 MB', value: 500 },
  { label: '1 GB', value: 1024 },
  { label: '5 GB', value: 5120 },
  { label: '10 GB', value: 10240 },
]

function ContainerCreationWizard({ visible, onClose, onSuccess }: ContainerCreationWizardProps) {
  const [currentStep, setCurrentStep] = useState(0)
  const [form] = Form.useForm<WizardFormData>()
  const [loading, setLoading] = useState(false)
  const [progress, setProgress] = useState(0)
  const [creationPhase, setCreationPhase] = useState('')


  const steps = [
    {
      title: 'Basic Configuration',
      icon: <InfoCircleOutlined />,
      description: 'Container name, size, and password',
    },
    {
      title: 'Encryption Settings',
      icon: <LockOutlined />,
      description: 'Choose encryption algorithm and parameters',
    },
    {
      title: 'Steganography Options',
      icon: <EyeInvisibleOutlined />,
      description: 'Hide container in carrier files (optional)',
    },
    {
      title: 'Review & Create',
      icon: <CheckCircleOutlined />,
      description: 'Review settings and create container',
    },
  ]

  const formatBytes = (mb: number) => {
    if (mb >= 1024) {
      return `${(mb / 1024).toFixed(1)} GB`
    }
    return `${mb} MB`
  }

  const validatePassword = (password: string) => {
    const minLength = password.length >= 8
    const hasUpper = /[A-Z]/.test(password)
    const hasLower = /[a-z]/.test(password)
    const hasNumber = /\d/.test(password)
    const hasSpecial = /[!@#$%^&*(),.?":{}|<>]/.test(password)
    
    return {
      minLength,
      hasUpper,
      hasLower,
      hasNumber,
      hasSpecial,
      score: [minLength, hasUpper, hasLower, hasNumber, hasSpecial].filter(Boolean).length,
    }
  }

  const getPasswordStrength = (score: number) => {
    if (score <= 2) return { text: 'Weak', color: 'red', percent: 25 }
    if (score === 3) return { text: 'Fair', color: 'orange', percent: 50 }
    if (score === 4) return { text: 'Good', color: 'blue', percent: 75 }
    return { text: 'Strong', color: 'green', percent: 100 }
  }

  const handleNext = async () => {
    try {
      await form.validateFields()
      setCurrentStep(prev => prev + 1)
    } catch (error) {
      console.error('Validation failed:', error)
    }
  }

  const handlePrevious = () => {
    setCurrentStep(prev => prev - 1)
  }

  const simulateProgress = useCallback((phases: string[]) => {
    let currentPhaseIndex = 0
    let currentProgress = 0
    
    const updateProgress = () => {
      if (currentPhaseIndex < phases.length) {
        setCreationPhase(phases[currentPhaseIndex])
        
        const increment = Math.random() * 15 + 5
        currentProgress = Math.min(currentProgress + increment, (currentPhaseIndex + 1) * (100 / phases.length))
        setProgress(currentProgress)
        
        if (currentProgress >= (currentPhaseIndex + 1) * (100 / phases.length)) {
          currentPhaseIndex++
          if (currentPhaseIndex < phases.length) {
            setTimeout(updateProgress, 500)
          } else {
            setProgress(100)
            setCreationPhase('Container created successfully!')
          }
        } else {
          setTimeout(updateProgress, 200 + Math.random() * 300)
        }
      }
    }
    
    updateProgress()
  }, [])

  const handleCreate = async () => {
    try {
      setLoading(true)
      setProgress(0)
      
      const values = await form.validateFields()
      
      // Simulate creation phases
      const phases = [
        'Initializing container structure...',
        'Generating encryption keys...',
        'Setting up file system...',
        'Applying encryption settings...',
      ]
      
      if (values.steganographic && values.carrierFile) {
        phases.push('Processing carrier file...')
        phases.push('Embedding container data...')
      }
      
      phases.push('Finalizing container...')
      
      simulateProgress(phases)
      
      const config: ContainerConfig = {
        name: values.name,
        size: values.size * 1024 * 1024, // Convert MB to bytes
        password: values.password,
        encryptionAlgorithm: values.encryptionAlgorithm,
        steganographic: values.steganographic,
        carrierFile: values.carrierFile?.name,
      }
      
      // Wait for progress simulation to complete
      await new Promise(resolve => {
        const checkProgress = () => {
          if (progress >= 100) {
            resolve(void 0)
          } else {
            setTimeout(checkProgress, 100)
          }
        }
        checkProgress()
      })
      
      await containerAPI.createContainer(config)
      
      message.success('Container created successfully!')
      onSuccess()
      handleClose()
      
    } catch (error) {
      console.error('Failed to create container:', error)
      message.error('Failed to create container')
    } finally {
      setLoading(false)
      setProgress(0)
      setCreationPhase('')
    }
  }

  const handleClose = () => {
    form.resetFields()
    setCurrentStep(0)
    setProgress(0)
    setCreationPhase('')
    setLoading(false)
    onClose()
  }

  const renderBasicConfiguration = () => (
    <motion.div
      initial={{ opacity: 0, x: 20 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: -20 }}
      transition={{ duration: 0.3 }}
    >
      <Card title="Basic Container Settings" className="mb-6">
        <Row gutter={[24, 24]}>
          <Col span={24}>
            <Form.Item
              name="name"
              label="Container Name"
              rules={[
                { required: true, message: 'Please enter a container name' },
                { min: 3, message: 'Name must be at least 3 characters' },
                { max: 50, message: 'Name must be less than 50 characters' },
                { pattern: /^[a-zA-Z0-9_-]+$/, message: 'Only letters, numbers, hyphens, and underscores allowed' },
              ]}
            >
              <Input
                placeholder="Enter container name"
                size="large"
                suffix={
                  <Tooltip title="Choose a descriptive name for your container">
                    <QuestionCircleOutlined />
                  </Tooltip>
                }
              />
            </Form.Item>
          </Col>
          
          <Col span={24}>
            <Form.Item
              name="size"
              label="Container Size (MB)"
              rules={[
                { required: true, message: 'Please specify container size' },
                { type: 'number', min: 10, message: 'Minimum size is 10 MB' },
                { type: 'number', max: 1048576, message: 'Maximum size is 1 TB' },
              ]}
              initialValue={1024}
            >
              <div>
                <Row gutter={16} className="mb-4">
                  {SIZE_PRESETS.map(preset => (
                    <Col key={preset.value}>
                      <Button
                        size="small"
                        onClick={() => form.setFieldValue('size', preset.value)}
                      >
                        {preset.label}
                      </Button>
                    </Col>
                  ))}
                </Row>
                <InputNumber
                  min={10}
                  max={1048576}
                  style={{ width: '100%' }}
                  size="large"
                  formatter={value => `${value} MB`}
                  parser={value => {
                    const parsed = value?.replace(' MB', '') || ''
                    return parseInt(parsed) || 0
                  }}
                  onChange={value => {
                    if (value) {
                      form.setFieldValue('size', value)
                    }
                  }}
                />
                <div className="mt-2 text-sm text-gray-500">
                  Estimated size: {formatBytes(Form.useWatch('size', form) || 1024)}
                </div>
              </div>
            </Form.Item>
          </Col>
        </Row>
      </Card>

      <Card title="Security Settings">
        <Row gutter={[24, 24]}>
          <Col span={12}>
            <Form.Item
              name="password"
              label="Master Password"
              rules={[
                { required: true, message: 'Please enter a password' },
                { min: 8, message: 'Password must be at least 8 characters' },
              ]}
            >
              <Input.Password
                placeholder="Enter master password"
                size="large"
                onChange={(e) => {
                  const validation = validatePassword(e.target.value)
                  // You could store validation state here if needed
                }}
              />
            </Form.Item>
            
            <Form.Item dependencies={['password']}>
              {({ getFieldValue }) => {
                const password = getFieldValue('password') || ''
                const validation = validatePassword(password)
                const strength = getPasswordStrength(validation.score)
                
                return password ? (
                  <div className="space-y-2">
                    <div className="flex items-center justify-between">
                      <span className="text-sm">Password Strength:</span>
                      <span className={`text-sm font-medium text-${strength.color}-500`}>
                        {strength.text}
                      </span>
                    </div>
                    <Progress
                      percent={strength.percent}
                      strokeColor={strength.color}
                      showInfo={false}
                      size="small"
                    />
                    <div className="text-xs space-y-1">
                      <div className={validation.minLength ? 'text-green-500' : 'text-gray-400'}>
                        ✓ At least 8 characters
                      </div>
                      <div className={validation.hasUpper ? 'text-green-500' : 'text-gray-400'}>
                        ✓ Uppercase letter
                      </div>
                      <div className={validation.hasLower ? 'text-green-500' : 'text-gray-400'}>
                        ✓ Lowercase letter
                      </div>
                      <div className={validation.hasNumber ? 'text-green-500' : 'text-gray-400'}>
                        ✓ Number
                      </div>
                      <div className={validation.hasSpecial ? 'text-green-500' : 'text-gray-400'}>
                        ✓ Special character
                      </div>
                    </div>
                  </div>
                ) : null
              }}
            </Form.Item>
          </Col>
          
          <Col span={12}>
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
                placeholder="Confirm master password"
                size="large"
              />
            </Form.Item>
          </Col>
        </Row>
      </Card>
    </motion.div>
  )

  const renderEncryptionSettings = () => (
    <motion.div
      initial={{ opacity: 0, x: 20 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: -20 }}
      transition={{ duration: 0.3 }}
    >
      <Card title="Encryption Configuration" className="mb-6">
        <Row gutter={[24, 24]}>
          <Col span={24}>
            <Form.Item
              name="encryptionAlgorithm"
              label="Encryption Algorithm"
              initialValue="AES-256-GCM"
              rules={[{ required: true, message: 'Please select an encryption algorithm' }]}
            >
              <Radio.Group size="large">
                {ENCRYPTION_ALGORITHMS.map(algo => (
                  <Radio.Button key={algo.value} value={algo.value} className="mb-2">
                    <div>
                      <div className="font-medium">{algo.label}</div>
                      <div className="text-xs text-gray-500">{algo.description}</div>
                    </div>
                  </Radio.Button>
                ))}
              </Radio.Group>
            </Form.Item>
          </Col>
        </Row>
      </Card>

      <Collapse
        ghost
        items={[
          {
            key: 'advanced',
            label: (
              <div className="flex items-center">
                <SettingOutlined className="mr-2" />
                Advanced Encryption Settings
              </div>
            ),
            children: (
              <Card>
                <Row gutter={[24, 24]}>
                  <Col span={24}>
                    <Form.Item
                      name="keyDerivationIterations"
                      label={
                        <span>
                          Key Derivation Iterations
                          <Tooltip title="Higher values increase security but slow down container mounting">
                            <QuestionCircleOutlined className="ml-2" />
                          </Tooltip>
                        </span>
                      }
                      initialValue={100000}
                    >
                      <Slider
                        min={50000}
                        max={1000000}
                        step={10000}
                        marks={{
                          50000: '50K (Fast)',
                          100000: '100K (Recommended)',
                          500000: '500K (Secure)',
                          1000000: '1M (Maximum)',
                        }}
                        tooltip={{
                          formatter: (value) => `${(value || 0) / 1000}K iterations`,
                        }}
                      />
                    </Form.Item>
                  </Col>
                  
                  <Col span={12}>
                    <Form.Item
                      name="compressionEnabled"
                      label="Enable Compression"
                      valuePropName="checked"
                      initialValue={true}
                    >
                      <Switch
                        checkedChildren="ON"
                        unCheckedChildren="OFF"
                      />
                    </Form.Item>
                    <Text type="secondary" className="text-sm">
                      Reduces container size but may impact performance
                    </Text>
                  </Col>
                  
                  <Col span={12}>
                    <Form.Item
                      name="integrityChecking"
                      label="Integrity Checking"
                      valuePropName="checked"
                      initialValue={true}
                    >
                      <Switch
                        checkedChildren="ON"
                        unCheckedChildren="OFF"
                      />
                    </Form.Item>
                    <Text type="secondary" className="text-sm">
                      Detects tampering and corruption
                    </Text>
                  </Col>
                </Row>
              </Card>
            ),
          },
        ]}
      />
    </motion.div>
  )

  const renderSteganographyOptions = () => (
    <motion.div
      initial={{ opacity: 0, x: 20 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: -20 }}
      transition={{ duration: 0.3 }}
    >
      <Card title="Steganography Settings">
        <Alert
          message="Steganography hides your encrypted container inside innocent-looking files"
          description="This adds an extra layer of security by making the container invisible to casual inspection."
          type="info"
          showIcon
          className="mb-6"
        />
        
        <Row gutter={[24, 24]}>
          <Col span={24}>
            <Form.Item
              name="steganographic"
              label="Enable Steganography"
              valuePropName="checked"
              initialValue={false}
            >
              <Switch
                size="default"
                checkedChildren="Enabled"
                unCheckedChildren="Disabled"
              />
            </Form.Item>
          </Col>
          
          <Form.Item dependencies={['steganographic']}>
            {({ getFieldValue }) => {
              const steganographicEnabled = getFieldValue('steganographic')
              
              return steganographicEnabled ? (
                <Col span={24}>
                  <motion.div
                    initial={{ opacity: 0, height: 0 }}
                    animate={{ opacity: 1, height: 'auto' }}
                    exit={{ opacity: 0, height: 0 }}
                    transition={{ duration: 0.3 }}
                  >
                    <Row gutter={[24, 24]}>
                      <Col span={12}>
                        <Form.Item
                          name="carrierType"
                          label="Carrier File Type"
                          rules={[{ required: true, message: 'Please select a carrier type' }]}
                        >
                          <Select
                            placeholder="Select carrier file type"
                            size="large"
                            options={CARRIER_TYPES.map(type => ({
                              ...type,
                              label: (
                                <div>
                                  <div className="font-medium">{type.label}</div>
                                  <div className="text-xs text-gray-500">{type.description}</div>
                                </div>
                              ),
                            }))}
                          />
                        </Form.Item>
                      </Col>
                      
                      <Col span={12}>
                        <Form.Item
                          name="embeddingDensity"
                          label={
                            <span>
                              Embedding Density
                              <Tooltip title="Higher density uses more of the carrier file capacity but may be more detectable">
                                <QuestionCircleOutlined className="ml-2" />
                              </Tooltip>
                            </span>
                          }
                          initialValue={25}
                        >
                          <Slider
                            min={10}
                            max={75}
                            marks={{
                              10: 'Low',
                              25: 'Medium',
                              50: 'High',
                              75: 'Maximum',
                            }}
                            tooltip={{
                              formatter: (value) => `${value}%`,
                            }}
                          />
                        </Form.Item>
                      </Col>
                      
                      <Col span={24}>
                        <Form.Item
                          name="carrierFile"
                          label="Carrier File"
                          rules={[{ required: true, message: 'Please upload a carrier file' }]}
                        >
                          <Dragger
                            name="carrierFile"
                            multiple={false}
                            beforeUpload={(file) => {
                              form.setFieldValue('carrierFile', file)
                              return false // Prevent automatic upload
                            }}
                            accept=".png,.jpg,.jpeg,.bmp,.wav"
                          >
                            <p className="ant-upload-drag-icon">
                              <UploadOutlined className="text-2xl" />
                            </p>
                            <p className="ant-upload-text">
                              Click or drag carrier file here
                            </p>
                            <p className="ant-upload-hint">
                              Support PNG, JPEG, BMP, and WAV files
                            </p>
                          </Dragger>
                        </Form.Item>
                      </Col>
                    </Row>
                  </motion.div>
                </Col>
              ) : null
            }}
          </Form.Item>
        </Row>
      </Card>
    </motion.div>
  )

  const renderReviewAndCreate = () => {
    const values = form.getFieldsValue()
    
    return (
      <motion.div
        initial={{ opacity: 0, x: 20 }}
        animate={{ opacity: 1, x: 0 }}
        exit={{ opacity: 0, x: -20 }}
        transition={{ duration: 0.3 }}
      >
        {loading ? (
          <Card title="Creating Container" className="text-center">
            <div className="py-8">
              <LoadingOutlined className="text-4xl text-blue-500 mb-4" />
              <Title level={4}>{creationPhase}</Title>
              <Progress
                percent={Math.round(progress)}
                strokeColor={{
                  '0%': '#0ea5e9',
                  '100%': '#06b6d4',
                }}
                className="mb-4"
              />
              <Text type="secondary">
                Please wait while your container is being created...
              </Text>
            </div>
          </Card>
        ) : (
          <div className="space-y-6">
            <Card title="Container Summary">
              <Row gutter={[24, 16]}>
                <Col span={12}>
                  <div>
                    <Text strong>Name:</Text>
                    <div>{values.name}</div>
                  </div>
                </Col>
                <Col span={12}>
                  <div>
                    <Text strong>Size:</Text>
                    <div>{formatBytes(values.size || 0)}</div>
                  </div>
                </Col>
                <Col span={12}>
                  <div>
                    <Text strong>Encryption:</Text>
                    <div>{values.encryptionAlgorithm}</div>
                  </div>
                </Col>
                <Col span={12}>
                  <div>
                    <Text strong>Steganography:</Text>
                    <div>{values.steganographic ? 'Enabled' : 'Disabled'}</div>
                  </div>
                </Col>
                {values.steganographic && (
                  <>
                    <Col span={12}>
                      <div>
                        <Text strong>Carrier Type:</Text>
                        <div>{values.carrierType}</div>
                      </div>
                    </Col>
                    <Col span={12}>
                      <div>
                        <Text strong>Embedding Density:</Text>
                        <div>{values.embeddingDensity}%</div>
                      </div>
                    </Col>
                  </>
                )}
              </Row>
            </Card>
            
            <Alert
              message="Ready to Create Container"
              description="Please review the settings above. Once created, some settings cannot be changed."
              type="success"
              showIcon
            />
          </div>
        )}
      </motion.div>
    )
  }

  const renderStepContent = () => {
    switch (currentStep) {
      case 0:
        return renderBasicConfiguration()
      case 1:
        return renderEncryptionSettings()
      case 2:
        return renderSteganographyOptions()
      case 3:
        return renderReviewAndCreate()
      default:
        return null
    }
  }

  return (
    <Modal
      title="Create New Container"
      open={visible}
      onCancel={handleClose}
      width={800}
      footer={null}
      destroyOnClose
      className="container-creation-wizard"
    >
      <div className="space-y-6">
        <Steps
          current={currentStep}
          items={steps}
          className="mb-8"
        />
        
        <Form
          form={form}
          layout="vertical"
          size="large"
          className="min-h-[400px]"
        >
          <AnimatePresence mode="wait">
            {renderStepContent()}
          </AnimatePresence>
        </Form>
        
        <Divider />
        
        <div className="flex justify-between">
          <Button
            size="large"
            onClick={handleClose}
            disabled={loading}
          >
            Cancel
          </Button>
          
          <Space>
            {currentStep > 0 && (
              <Button
                size="large"
                onClick={handlePrevious}
                disabled={loading}
              >
                Previous
              </Button>
            )}
            
            {currentStep < steps.length - 1 ? (
              <Button
                type="primary"
                size="large"
                onClick={handleNext}
                disabled={loading}
              >
                Next
              </Button>
            ) : (
              <Button
                type="primary"
                size="large"
                onClick={handleCreate}
                loading={loading}
                icon={<CheckCircleOutlined />}
              >
                Create Container
              </Button>
            )}
          </Space>
        </div>
      </div>
    </Modal>
  )
}

export default ContainerCreationWizard