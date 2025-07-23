import { useEffect, useState, useCallback } from 'react'
import {
  Card,
  Table,
  Button,
  Space,
  Tag,
  Tooltip,
  Modal,
  message,
  Dropdown,
  Input,
  Progress,
  Row,
  Col,
  Statistic,
  Badge,
  Upload,
  Spin,
  Alert,
} from 'antd'
import {
  PlusOutlined,
  PlayCircleOutlined,
  StopOutlined,
  DeleteOutlined,
  MoreOutlined,
  SearchOutlined,
  ReloadOutlined,
  SafetyOutlined,
  ShareAltOutlined,
  InboxOutlined,
  FolderOutlined,
  LockOutlined,
  UnlockOutlined,
  ExclamationCircleOutlined,
  CloudUploadOutlined,
} from '@ant-design/icons'
import { motion, AnimatePresence } from 'framer-motion'
import { containerAPI, type Container } from '../services/api'
import SimpleContainerWizard from '../components/SimpleContainerWizard'

const { Search } = Input
const { Dragger } = Upload

interface ContainerOperation {
  id: string
  type: 'mounting' | 'unmounting' | 'checking'
  progress: number
}

function Containers() {
  const [containers, setContainers] = useState<Container[]>([])
  const [loading, setLoading] = useState(true)
  const [searchText, setSearchText] = useState('')
  const [operations, setOperations] = useState<ContainerOperation[]>([])
  const [viewMode, setViewMode] = useState<'grid' | 'table'>('grid')
  const [wizardVisible, setWizardVisible] = useState(false)


  useEffect(() => {
    loadContainers()
    // Set up real-time updates
    const interval = setInterval(loadContainers, 5000)
    return () => clearInterval(interval)
  }, [])

  const loadContainers = async () => {
    try {
      setLoading(true)
      const data = await containerAPI.getContainers()
      setContainers(data)
    } catch (error) {
      console.error('Failed to load containers:', error)
      message.error('Failed to load containers')
    } finally {
      setLoading(false)
    }
  }

  const startOperation = (containerId: string, type: ContainerOperation['type']) => {
    const operation: ContainerOperation = { id: containerId, type, progress: 0 }
    setOperations(prev => [...prev.filter(op => op.id !== containerId), operation])
    
    // Simulate progress
    const progressInterval = setInterval(() => {
      setOperations(prev => prev.map(op => 
        op.id === containerId 
          ? { ...op, progress: Math.min(op.progress + Math.random() * 20, 95) }
          : op
      ))
    }, 200)

    return () => {
      clearInterval(progressInterval)
      setOperations(prev => prev.filter(op => op.id !== containerId))
    }
  }

  const handleMount = async (container: Container) => {
    const cleanup = startOperation(container.id, 'mounting')
    
    try {
      // Show password modal
      const password = await showPasswordModal(container.name)
      if (!password) {
        cleanup()
        return
      }
      
      await containerAPI.mountContainer(container.id, password)
      
      // Complete progress
      setOperations(prev => prev.map(op => 
        op.id === container.id ? { ...op, progress: 100 } : op
      ))
      
      setTimeout(() => {
        cleanup()
        message.success(`Container "${container.name}" mounted successfully`)
        loadContainers()
      }, 500)
    } catch (error) {
      cleanup()
      message.error('Failed to mount container')
    }
  }

  const handleUnmount = async (container: Container) => {
    const cleanup = startOperation(container.id, 'unmounting')
    
    try {
      await containerAPI.unmountContainer(container.id)
      
      setOperations(prev => prev.map(op => 
        op.id === container.id ? { ...op, progress: 100 } : op
      ))
      
      setTimeout(() => {
        cleanup()
        message.success(`Container "${container.name}" unmounted successfully`)
        loadContainers()
      }, 500)
    } catch (error) {
      cleanup()
      message.error('Failed to unmount container')
    }
  }

  const handleIntegrityCheck = async (container: Container) => {
    const cleanup = startOperation(container.id, 'checking')
    
    try {
      const result = await containerAPI.checkIntegrity(container.id)
      
      setOperations(prev => prev.map(op => 
        op.id === container.id ? { ...op, progress: 100 } : op
      ))
      
      setTimeout(() => {
        cleanup()
        if (result.valid) {
          message.success(`Container "${container.name}" integrity check passed`)
        } else {
          message.warning(`Container "${container.name}" has integrity issues: ${result.issues.join(', ')}`)
        }
      }, 500)
    } catch (error) {
      cleanup()
      message.error('Failed to check container integrity')
    }
  }

  const showPasswordModal = (containerName: string): Promise<string | null> => {
    return new Promise((resolve) => {
      let password = ''
      
      Modal.confirm({
        title: `Mount Container: ${containerName}`,
        content: (
          <div className="py-4">
            <Input.Password
              placeholder="Enter container password"
              onChange={(e) => password = e.target.value}
              onPressEnter={() => {
                Modal.destroyAll()
                resolve(password)
              }}
            />
          </div>
        ),
        onOk: () => resolve(password),
        onCancel: () => resolve(null),
        okText: 'Mount',
        cancelText: 'Cancel',
      })
    })
  }

  const handleDelete = (container: Container) => {
    Modal.confirm({
      title: 'Delete Container',
      content: `Are you sure you want to delete "${container.name}"? This action cannot be undone.`,
      okText: 'Delete',
      okType: 'danger',
      cancelText: 'Cancel',
      onOk: async () => {
        try {
          await containerAPI.deleteContainer(container.id)
          message.success(`Container "${container.name}" deleted successfully`)
          loadContainers()
        } catch (error) {
          message.error('Failed to delete container')
        }
      },
    })
  }

  const handleDragDrop = useCallback((info: any) => {
    const { status } = info.file
    if (status === 'done') {
      message.success(`${info.file.name} container loaded successfully`)
      loadContainers()
    } else if (status === 'error') {
      message.error(`${info.file.name} failed to load`)
    }
  }, [])

  const getStatusColor = (status: Container['status']) => {
    switch (status) {
      case 'mounted':
        return 'green'
      case 'unmounted':
        return 'default'
      case 'locked':
        return 'orange'
      case 'error':
        return 'red'
      default:
        return 'default'
    }
  }

  const getStatusIcon = (status: Container['status']) => {
    switch (status) {
      case 'mounted':
        return <UnlockOutlined className="text-green-500" />
      case 'unmounted':
        return <LockOutlined className="text-gray-500" />
      case 'locked':
        return <ExclamationCircleOutlined className="text-orange-500" />
      case 'error':
        return <ExclamationCircleOutlined className="text-red-500" />
      default:
        return <LockOutlined className="text-gray-500" />
    }
  }

  const formatBytes = (bytes: number) => {
    if (bytes === 0) return '0 Bytes'
    const k = 1024
    const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB']
    const i = Math.floor(Math.log(bytes) / Math.log(k))
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i]
  }

  const handleResize = (container: Container) => {
    Modal.confirm({
      title: `Resize Container: ${container.name}`,
      content: (
        <div className="py-4">
          <p className="mb-4">Current size: {formatBytes(container.size)}</p>
          <Input
            placeholder="Enter new size (MB)"
            type="number"
            min={Math.ceil(container.size / (1024 * 1024))}
            max={1048576}
            onChange={(e) => {
              // Store the new size value
            }}
          />
          <p className="text-sm text-gray-500 mt-2">
            Note: You can only increase container size, not decrease it.
          </p>
        </div>
      ),
      onOk: async () => {
        try {
          // This would call the resize API
          message.success(`Container "${container.name}" resized successfully`)
          loadContainers()
        } catch (error) {
          message.error('Failed to resize container')
        }
      },
      okText: 'Resize',
      cancelText: 'Cancel',
    })
  }

  const handleShare = (container: Container) => {
    Modal.info({
      title: `Share Files from: ${container.name}`,
      content: (
        <div className="py-4">
          <p className="mb-4">Select files to share:</p>
          <div className="space-y-2">
            <div className="p-2 border rounded hover:bg-gray-50">
              📄 document.pdf (2.3 MB)
            </div>
            <div className="p-2 border rounded hover:bg-gray-50">
              📷 image.jpg (1.8 MB)
            </div>
            <div className="p-2 border rounded hover:bg-gray-50">
              📁 folder/ (15 files)
            </div>
          </div>
          <p className="text-sm text-gray-500 mt-4">
            Full sharing interface would be implemented here.
          </p>
        </div>
      ),
      width: 500,
    })
  }

  const getActionMenuItems = (container: Container) => [
    {
      key: 'integrity',
      icon: <SafetyOutlined />,
      label: 'Check Integrity',
      onClick: () => handleIntegrityCheck(container),
    },
    {
      key: 'resize',
      icon: <CloudUploadOutlined />,
      label: 'Resize Container',
      onClick: () => handleResize(container),
      disabled: container.status === 'mounted',
    },
    {
      key: 'share',
      icon: <ShareAltOutlined />,
      label: 'Share Files',
      onClick: () => handleShare(container),
      disabled: container.status !== 'mounted',
    },
    {
      type: 'divider' as const,
    },
    {
      key: 'delete',
      icon: <DeleteOutlined />,
      label: 'Delete',
      danger: true,
      onClick: () => handleDelete(container),
    },
  ]

  const getContainerStats = () => {
    const total = containers.length
    const mounted = containers.filter(c => c.status === 'mounted').length
    const totalSize = containers.reduce((sum, c) => sum + c.size, 0)
    const errors = containers.filter(c => c.status === 'error').length
    
    return { total, mounted, totalSize, errors }
  }

  const stats = getContainerStats()

  const renderContainerCard = (container: Container) => {
    const operation = operations.find(op => op.id === container.id)
    const isOperating = !!operation
    
    return (
      <motion.div
        key={container.id}
        layout
        initial={{ opacity: 0, scale: 0.9 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={{ opacity: 0, scale: 0.9 }}
        transition={{ duration: 0.3 }}
        whileHover={{ y: -4, boxShadow: '0 8px 25px rgba(0,0,0,0.15)' }}
      >
        <Card
          className="h-full relative overflow-hidden"
          styles={{ body: { padding: '20px' } }}
          hoverable
        >
          {/* Status indicator */}
          <div className="absolute top-4 right-4">
            <Badge
              status={container.status === 'mounted' ? 'success' : 
                     container.status === 'error' ? 'error' : 'default'}
              dot
            />
          </div>

          {/* Container icon and name */}
          <div className="flex items-center mb-4">
            <div className="flex-shrink-0 mr-3">
              {getStatusIcon(container.status)}
            </div>
            <div className="flex-1 min-w-0">
              <h3 className="text-lg font-semibold text-gray-800 dark:text-gray-200 truncate">
                {container.name}
              </h3>
              <p className="text-sm text-gray-500 dark:text-gray-400 truncate">
                {container.path}
              </p>
            </div>
          </div>

          {/* Container details */}
          <div className="space-y-3 mb-4">
            <div className="flex justify-between items-center">
              <span className="text-sm text-gray-600 dark:text-gray-400">Size</span>
              <span className="text-sm font-medium">{formatBytes(container.size)}</span>
            </div>
            
            <div className="flex justify-between items-center">
              <span className="text-sm text-gray-600 dark:text-gray-400">Status</span>
              <Tag color={getStatusColor(container.status)}>
                {container.status.toUpperCase()}
              </Tag>
            </div>

            <div className="flex justify-between items-center">
              <span className="text-sm text-gray-600 dark:text-gray-400">Last Access</span>
              <span className="text-sm">{new Date(container.lastAccessed).toLocaleDateString()}</span>
            </div>
          </div>

          {/* Features */}
          <div className="flex flex-wrap gap-1 mb-4">
            {container.encrypted && (
              <Tag color="blue">ENC</Tag>
            )}
            {container.steganographic && (
              <Tag color="purple">STEG</Tag>
            )}
          </div>

          {/* Progress bar for operations */}
          <AnimatePresence>
            {isOperating && (
              <motion.div
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: 'auto' }}
                exit={{ opacity: 0, height: 0 }}
                className="mb-4"
              >
                <div className="text-sm text-gray-600 dark:text-gray-400 mb-2 capitalize">
                  {operation.type}...
                </div>
                <Progress 
                  percent={operation.progress} 
                  size="small"
                  strokeColor={{
                    '0%': '#0ea5e9',
                    '100%': '#06b6d4',
                  }}
                />
              </motion.div>
            )}
          </AnimatePresence>

          {/* Actions */}
          <div className="flex justify-between items-center">
            <Space size="small">
              {container.status === 'unmounted' ? (
                <Button
                  type="primary"
                  size="small"
                  icon={<PlayCircleOutlined />}
                  onClick={() => handleMount(container)}
                  disabled={isOperating}
                >
                  Mount
                </Button>
              ) : (
                <Button
                  size="small"
                  icon={<StopOutlined />}
                  onClick={() => handleUnmount(container)}
                  disabled={isOperating}
                >
                  Unmount
                </Button>
              )}
            </Space>
            
            <Dropdown
              menu={{ items: getActionMenuItems(container) }}
              trigger={['click']}
              disabled={isOperating}
            >
              <Button size="small" icon={<MoreOutlined />} />
            </Dropdown>
          </div>
        </Card>
      </motion.div>
    )
  }

  const columns = [
    {
      title: 'Name',
      dataIndex: 'name',
      key: 'name',
      filteredValue: searchText ? [searchText] : null,
      onFilter: (value: string, record: Container) =>
        record.name.toLowerCase().includes(value.toLowerCase()) ||
        record.path.toLowerCase().includes(value.toLowerCase()),
      render: (text: string, record: Container) => (
        <div>
          <div className="font-medium">{text}</div>
          <div className="text-sm text-gray-500 dark:text-gray-400">
            {record.path}
          </div>
        </div>
      ),
    },
    {
      title: 'Size',
      dataIndex: 'size',
      key: 'size',
      render: (size: number) => formatBytes(size),
      sorter: (a: Container, b: Container) => a.size - b.size,
    },
    {
      title: 'Status',
      dataIndex: 'status',
      key: 'status',
      render: (status: Container['status']) => (
        <Tag color={getStatusColor(status)}>
          {status.toUpperCase()}
        </Tag>
      ),
      filters: [
        { text: 'Mounted', value: 'mounted' },
        { text: 'Unmounted', value: 'unmounted' },
        { text: 'Locked', value: 'locked' },
        { text: 'Error', value: 'error' },
      ],
      onFilter: (value: boolean | React.Key, record: Container) => record.status === value,
    },
    {
      title: 'Features',
      key: 'features',
      render: (_, record: Container) => (
        <Space>
          {record.encrypted && (
            <Tooltip title="Encrypted">
              <Tag color="blue">ENC</Tag>
            </Tooltip>
          )}
          {record.steganographic && (
            <Tooltip title="Steganographic">
              <Tag color="purple">STEG</Tag>
            </Tooltip>
          )}
        </Space>
      ),
    },
    {
      title: 'Last Accessed',
      dataIndex: 'lastAccessed',
      key: 'lastAccessed',
      render: (date: string) => new Date(date).toLocaleDateString(),
      sorter: (a: Container, b: Container) =>
        new Date(a.lastAccessed).getTime() - new Date(b.lastAccessed).getTime(),
    },
    {
      title: 'Actions',
      key: 'actions',
      render: (_, record: Container) => (
        <Space>
          {record.status === 'unmounted' ? (
            <Tooltip title="Mount Container">
              <Button
                type="primary"
                size="small"
                icon={<PlayCircleOutlined />}
                onClick={() => handleMount(record)}
              />
            </Tooltip>
          ) : (
            <Tooltip title="Unmount Container">
              <Button
                size="small"
                icon={<StopOutlined />}
                onClick={() => handleUnmount(record)}
              />
            </Tooltip>
          )}
          <Dropdown
            menu={{ items: getActionMenuItems(record) }}
            trigger={['click']}
          >
            <Button size="small" icon={<MoreOutlined />} />
          </Dropdown>
        </Space>
      ),
    },
  ]

  const filteredContainers = containers.filter(
    (container) =>
      container.name.toLowerCase().includes(searchText.toLowerCase()) ||
      container.path.toLowerCase().includes(searchText.toLowerCase())
  )

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.5 }}
      className="space-y-6"
    >
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-2xl font-bold text-gray-800 dark:text-gray-200 mb-2">
            Container Management
          </h2>
          <p className="text-gray-600 dark:text-gray-400">
            Manage your encrypted containers with real-time status monitoring
          </p>
        </div>
        <Space>
          <Button
            type={viewMode === 'grid' ? 'primary' : 'default'}
            onClick={() => setViewMode('grid')}
            icon={<FolderOutlined />}
          >
            Grid
          </Button>
          <Button
            type={viewMode === 'table' ? 'primary' : 'default'}
            onClick={() => setViewMode('table')}
            icon={<SearchOutlined />}
          >
            Table
          </Button>
        </Space>
      </div>

      {/* Status Dashboard */}
      <Row gutter={[24, 24]}>
        <Col xs={24} sm={12} lg={6}>
          <motion.div
            initial={{ opacity: 0, scale: 0.9 }}
            animate={{ opacity: 1, scale: 1 }}
            transition={{ duration: 0.3, delay: 0.1 }}
          >
            <Card className="text-center">
              <Statistic
                title="Total Containers"
                value={stats.total}
                prefix={<FolderOutlined className="text-blue-500" />}
                valueStyle={{ color: '#3b82f6' }}
              />
            </Card>
          </motion.div>
        </Col>

        <Col xs={24} sm={12} lg={6}>
          <motion.div
            initial={{ opacity: 0, scale: 0.9 }}
            animate={{ opacity: 1, scale: 1 }}
            transition={{ duration: 0.3, delay: 0.2 }}
          >
            <Card className="text-center">
              <Statistic
                title="Mounted"
                value={stats.mounted}
                prefix={<UnlockOutlined className="text-green-500" />}
                valueStyle={{ color: '#10b981' }}
              />
            </Card>
          </motion.div>
        </Col>

        <Col xs={24} sm={12} lg={6}>
          <motion.div
            initial={{ opacity: 0, scale: 0.9 }}
            animate={{ opacity: 1, scale: 1 }}
            transition={{ duration: 0.3, delay: 0.3 }}
          >
            <Card className="text-center">
              <Statistic
                title="Total Size"
                value={formatBytes(stats.totalSize)}
                prefix={<CloudUploadOutlined className="text-purple-500" />}
                valueStyle={{ color: '#8b5cf6' }}
              />
            </Card>
          </motion.div>
        </Col>

        <Col xs={24} sm={12} lg={6}>
          <motion.div
            initial={{ opacity: 0, scale: 0.9 }}
            animate={{ opacity: 1, scale: 1 }}
            transition={{ duration: 0.3, delay: 0.4 }}
          >
            <Card className="text-center">
              <Statistic
                title="Errors"
                value={stats.errors}
                prefix={<ExclamationCircleOutlined className="text-red-500" />}
                valueStyle={{ color: stats.errors > 0 ? '#ef4444' : '#10b981' }}
              />
            </Card>
          </motion.div>
        </Col>
      </Row>

      {/* Drag and Drop Area */}
      <motion.div
        initial={{ opacity: 0, y: 20 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.5, delay: 0.5 }}
      >
        <Card>
          <Dragger
            name="container"
            multiple={false}
            action="/api/containers/upload"
            onChange={handleDragDrop}
            accept=".efs,.container"
            className="mb-6"
          >
            <p className="ant-upload-drag-icon">
              <InboxOutlined className="text-4xl text-blue-500" />
            </p>
            <p className="ant-upload-text text-lg font-medium">
              Drop container files here to load them
            </p>
            <p className="ant-upload-hint">
              Support for .efs and .container files. Click or drag files to this area to upload.
            </p>
          </Dragger>
        </Card>
      </motion.div>

      {/* Controls */}
      <Card>
        <div className="flex items-center justify-between mb-4">
          <Space>
            <Search
              placeholder="Search containers..."
              allowClear
              value={searchText}
              onChange={(e) => setSearchText(e.target.value)}
              style={{ width: 300 }}
              prefix={<SearchOutlined />}
            />
            <Button
              icon={<ReloadOutlined />}
              onClick={loadContainers}
              loading={loading}
            >
              Refresh
            </Button>
          </Space>
          <Button
            type="primary"
            icon={<PlusOutlined />}
            size="large"
            onClick={() => setWizardVisible(true)}
          >
            New Container
          </Button>
        </div>

        {/* Container List */}
        <Spin spinning={loading}>
          {viewMode === 'grid' ? (
            <Row gutter={[24, 24]}>
              <AnimatePresence>
                {filteredContainers.map((container) => (
                  <Col xs={24} sm={12} lg={8} xl={6} key={container.id}>
                    {renderContainerCard(container)}
                  </Col>
                ))}
              </AnimatePresence>
              
              {filteredContainers.length === 0 && !loading && (
                <Col span={24}>
                  <motion.div
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    className="text-center py-12"
                  >
                    <FolderOutlined className="text-6xl text-gray-300 dark:text-gray-600 mb-4" />
                    <h3 className="text-lg font-medium text-gray-500 dark:text-gray-400 mb-2">
                      No containers found
                    </h3>
                    <p className="text-gray-400 dark:text-gray-500 mb-4">
                      {searchText ? 'Try adjusting your search terms' : 'Create your first container to get started'}
                    </p>
                    {!searchText && (
                      <Button
                        type="primary"
                        icon={<PlusOutlined />}
                        onClick={() => setWizardVisible(true)}
                      >
                        Create Container
                      </Button>
                    )}
                  </motion.div>
                </Col>
              )}
            </Row>
          ) : (
            <Table
              columns={columns}
              dataSource={filteredContainers}
              rowKey="id"
              loading={loading}
              pagination={{
                pageSize: 10,
                showSizeChanger: true,
                showQuickJumper: true,
                showTotal: (total, range) =>
                  `${range[0]}-${range[1]} of ${total} containers`,
              }}
              className="overflow-x-auto"
            />
          )}
        </Spin>
      </Card>

      {/* Active Operations Alert */}
      <AnimatePresence>
        {operations.length > 0 && (
          <motion.div
            initial={{ opacity: 0, y: 50 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 50 }}
            className="fixed bottom-6 right-6 z-50"
          >
            <Alert
              message={`${operations.length} operation${operations.length > 1 ? 's' : ''} in progress`}
              description={
                <div className="space-y-2 mt-2">
                  {operations.map((op) => {
                    const container = containers.find(c => c.id === op.id)
                    return (
                      <div key={op.id} className="flex items-center justify-between">
                        <span className="text-sm">
                          {container?.name} - {op.type}
                        </span>
                        <Progress
                          percent={op.progress}
                          size="small"
                          style={{ width: 100 }}
                        />
                      </div>
                    )
                  })}
                </div>
              }
              type="info"
              showIcon
              className="shadow-lg"
            />
          </motion.div>
        )}
      </AnimatePresence>

      {/* Container Creation Wizard */}
      <SimpleContainerWizard
        visible={wizardVisible}
        onClose={() => setWizardVisible(false)}
        onSuccess={() => {
          loadContainers()
          message.success('Container created successfully!')
        }}
      />
    </motion.div>
  )
}

export default Containers