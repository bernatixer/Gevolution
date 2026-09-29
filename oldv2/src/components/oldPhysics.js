var moveForward = false;
var moveBackward = false;
var moveLeft = false;
var moveRight = false;

function animate() {
    requestAnimationFrame(animate);
    const delta = clock.getDelta();
    for (let i=0; i<entities.length; ++i) {
      moveEntity(entities[i], delta);
    }
    controls.update();
    render();
}

function moveEntity(entity, delta) {
    if (entity) {
      entity.userData.velocity.x -= entity.userData.velocity.x * 20.0 * delta;
      entity.userData.velocity.z -= entity.userData.velocity.z * 20.0 * delta;
  
      // entity.userData.direction.z = Number( moveForward ) - Number( moveBackward );
      // entity.userData.direction.x = Number( moveRight ) - Number( moveLeft );
      // entity.userData.direction.normalize();
  
      // if ( moveForward || moveBackward ) entity.userData.velocity.z -= entity.userData.direction.z * 400.0 * delta;
      // if ( moveLeft || moveRight ) entity.userData.velocity.x -= entity.userData.direction.x * 400.0 * delta;
      if ( moveForward ) entity.userData.velocity.z -= 400.0 * delta;
  
      if (!isInGround(entity)) {
        entity.userData.velocity.y -= 9.81 * 5 * delta; // 10 = mass
        canJump = false;
      } else {
        canJump = true;
      }
  
      entity.translateX(-entity.userData.velocity.x * delta);
      entity.translateZ(entity.userData.velocity.z * delta);
      entity.translateY(entity.userData.velocity.y * delta);
      /*let translated = new THREE.Vector3(0,0,0);
      entity.getWorldDirection(translated);
      let defaultGravity;
      if (translated.y) {
        defaultGravity = translated.add(new THREE.Vector3(0,-1,1));
      } else {
        defaultGravity = new THREE.Vector3(0,1,0);
      }
      entity.translateOnAxis(defaultGravity, entity.userData.velocity.y * delta)*/
      
      if (!canMove(entity)) {
        // Intentem moure en eix Z-Y
        entity.translateX(entity.userData.velocity.x * delta);
        entity.userData.velocity.x = 0;
  
        // Intentem moure en eix Y
        if (!canMove(entity)) {
          entity.translateZ(-entity.userData.velocity.z * delta);
          entity.userData.velocity.z = 0;
  
          // No es pot moure
          if (!canMove(entity)) {
            entity.translateY(-entity.userData.velocity.y * delta);
            entity.userData.velocity.y = Math.max( 0, entity.userData.velocity.y );
          }
        }
      }
  
      // polloHelper.update();
    }
  }
  
  function canMove(entity) {
    let canMove = true;
    const vertices = getBoxVertices(entity);
    vertices.forEach(vertex => {
      if (world.getVoxel(...vertex)) {
        canMove = false;
        return;
      }
    });
    return canMove;
  }
  
  function isInGround(entity) {
    let inGround = false;
    const vertices = getLowerVertices(entity);
    vertices.forEach(vertex => {
      if (world.getVoxel(...vertex)) {
        inGround = true;
        return;
      }
    });
    return inGround;
  }
  
  function getBoxVertices(object) {
    const box = new THREE.Box3().setFromObject(object);
    return [
      [ Math.floor(box.min.x), Math.floor(box.min.y), Math.floor(box.min.z) ],
      [ Math.floor(box.min.x), Math.floor(box.min.y), Math.floor(box.max.z) ],
      [ Math.floor(box.min.x), Math.floor(box.max.y), Math.floor(box.min.z) ],
      [ Math.floor(box.min.x), Math.floor(box.max.y), Math.floor(box.max.z) ],
      [ Math.floor(box.max.x), Math.floor(box.min.y), Math.floor(box.min.z) ],
      [ Math.floor(box.max.x), Math.floor(box.min.y), Math.floor(box.max.z) ],
      [ Math.floor(box.max.x), Math.floor(box.max.y), Math.floor(box.min.z) ],
      [ Math.floor(box.max.x), Math.floor(box.max.y), Math.floor(box.max.z) ],
    ];
  }
  
  function getLowerVertices(object) {
    const box = new THREE.Box3().setFromObject(object);
    return [
      [ Math.floor(box.min.x), Math.floor(box.min.y-0.1), Math.floor(box.min.z) ],
      [ Math.floor(box.min.x), Math.floor(box.min.y-0.1), Math.floor(box.max.z) ],
      [ Math.floor(box.max.x), Math.floor(box.min.y-0.1), Math.floor(box.min.z) ],
      [ Math.floor(box.max.x), Math.floor(box.min.y-0.1), Math.floor(box.max.z) ],
    ];
  }

var onKeyDown = function ( event ) {
    switch ( event.keyCode ) {
      case 87: // w
        moveForward = true;
        break;
  
      case 65: // a
        moveLeft = true;
        break;
  
      case 83: // s
        moveBackward = true;
        break;
  
      case 68: // d
        moveRight = true;
        break;

    }
  };
  
  var onKeyUp = function ( event ) {
    switch ( event.keyCode ) {
      case 87: // w
        moveForward = false;
        break;
  
      case 65: // a
        moveLeft = false;
        break;
  
      case 83: // s
        moveBackward = false;
        break;
  
      case 68: // d
        moveRight = false;
        break;
    }
  };

document.addEventListener('keydown', onKeyDown, false);
document.addEventListener('keyup', onKeyUp, false);