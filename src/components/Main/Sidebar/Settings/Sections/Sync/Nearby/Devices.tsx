import type { NearbyDevice } from '@/api/nearby'
import DeviceRow from './DeviceRow'
import Searching from './Searching'

interface Props {
  devices: NearbyDevice[]
}

export default function Devices({ devices }: Props) {
  if (devices.length === 0) return <Searching />
  return devices.map(device => <DeviceRow key={device.id} device={device} />)
}
