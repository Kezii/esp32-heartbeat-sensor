use esp32_nimble::{uuid128, BLEClient, BLEDevice, BLEError, BLEScan};

use log::{error, info};
use LD24xx::{
    ld2450::{Ld2450TargetData, TargetData},
    RadarLLFrame,
};

pub struct RadarBle<'a> {
    characteristic: &'a mut esp32_nimble::BLERemoteCharacteristic,
}

impl<'a> RadarBle<'a> {
    pub async fn new(
        ble_device: &'a BLEDevice,
        client: &'a mut BLEClient,
    ) -> Result<Self, BLEError> {
        let mut ble_scan = BLEScan::new();

        info!("Scanning for BLE devices...");

        let device = ble_scan
            .active_scan(true)
            .interval(100)
            .window(99)
            .start(ble_device, 1000, |device, data| {
                if let Some(name) = data.name() {
                    info!("Found device: {:?}", name);
                    if name.starts_with(b"HLK-LD2450_") {
                        return Some(*device);
                    }
                }
                None
            })
            .await?;

        if let Some(device) = device {
            client.on_connect(|client| {
                client.update_conn_params(120, 120, 0, 60).unwrap();
            });

            client.connect(&device.addr()).await?;

            for service in client.get_services().await? {
                info!("Service: {:?}", service.uuid());
            }

            let service = client
                .get_service(uuid128!("0000fff0-0000-1000-8000-00805f9b34fb"))
                .await?;

            for characteristic in service.get_characteristics().await? {
                info!("Characteristic: {:?}", characteristic.uuid());
            }

            let characteristic = service
                .get_characteristic(uuid128!("0000fff1-0000-1000-8000-00805f9b34fb"))
                .await?;

            for descriptor in characteristic.get_descriptors().await? {
                info!("Descriptor: {:?}", descriptor.uuid());
            }

            if !characteristic.can_notify() {
                error!("Characteristic does not support notifications");
                return Err(BLEError::fail().unwrap_err());
            }

            Ok(Self { characteristic })
        } else {
            error!("No device found");
            Err(BLEError::fail().unwrap_err())
        }
    }


    pub async fn notify_data(
        &mut self,
        sender: std::sync::mpsc::SyncSender<TargetData>,
    ) -> Result<(), BLEError> {
        info!("Subscribing to notifications");
        self.characteristic
            .on_notify(move |data| {
                let frame = RadarLLFrame::deserialize(data);

                if let Some(RadarLLFrame::TargetFrame2D(frame)) = frame {
                    let data = Ld2450TargetData::deserialize(&frame);
                    // y parte da zero se sei davanti, va verso l'infinito
                    // x è negativo se sei a dx del modulo, positivo se sei a sx
                    if let Some(data) = data {
                        //info!("{:#?}", data);
                        for target in data.targets {
                            info!("target x: {}, y: {}", target.position.x, target.position.y);
                            sender.send(target).unwrap();
                        }
                    }
                }
            })
            .subscribe_notify(true)
            .await?;
        Ok(())
    }
}
